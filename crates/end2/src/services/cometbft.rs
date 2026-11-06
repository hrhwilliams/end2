use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use base64::{Engine, prelude::BASE64_STANDARD, prelude::BASE64_STANDARD_NO_PAD};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, PgConnection, QueryDsl, RunQueryDsl,
    SelectableHelper, r2d2::ConnectionManager,
};
use ed25519_dalek::{Signer, SigningKey};
use r2d2::Pool;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use vodozemac::{Curve25519PublicKey, Ed25519PublicKey};

use crate::{
    AppError, Device, DeviceId, DeviceKeyService, DirectoryKey, HistoricalKey,
    InboundAuthorization, InboundDevice, NewDevice, User,
    schema::{device, user as user_table},
};

// These structs mirror the ones in end2-cometbft/src/main.rs and must stay in sync.
// Binary fields are unpadded base64 strings.
#[derive(Serialize, Deserialize)]
struct KeyPayload {
    user_hash: String,
    // String to simplify passing to cometbft
    device_id: String,
    x25519: String,
    ed25519: String,
    // base64 device self-signature (ed25519 over x25519||ed25519 bytes)
    signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorization: Option<InboundAuthorization>,
}

#[derive(Serialize, Deserialize)]
struct KeyUploadTx {
    payload: KeyPayload,
    signature: String,
}

#[derive(Deserialize)]
struct AbciDeviceKeys {
    x25519: [u8; 32],
    ed25519: [u8; 32],
}

#[derive(Serialize)]
struct JsonRpcRequest<P: Serialize> {
    jsonrpc: &'static str,
    id: u32,
    method: &'static str,
    params: P,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum JsonRpcResponse<R> {
    Ok { result: R },
    Err { error: serde_json::Value },
}

#[derive(Deserialize)]
struct BroadcastSyncResult {
    code: u32,
    log: String,
    hash: String,
}

#[derive(Serialize)]
struct TxQueryParams {
    hash: String,
    prove: bool,
}

#[derive(Deserialize)]
struct AbciQueryResult {
    response: AbciQueryResponse,
}

#[derive(Deserialize)]
struct AbciQueryResponse {
    code: u32,
    log: String,
    value: Option<String>,
}

#[derive(Serialize)]
struct BroadcastParams {
    tx: String,
}

#[derive(Serialize)]
struct AbciQueryParams {
    path: String,
    data: String,
    prove: bool,
}

#[derive(Serialize)]
struct TxSearchParams {
    query: String,
    prove: bool,
    page: String,
    per_page: String,
    order_by: String,
}

#[derive(Deserialize)]
struct TxSearchResult {
    txs: Vec<TxResultInfo>,
}

#[derive(Deserialize)]
struct TxResultInfo {
    height: String, // CometBFT returns block height as a string
    tx: String,     // base64-encoded raw tx bytes
}

#[derive(Clone)]
pub struct CometBftDeviceKeyService {
    http: Client,
    rpc_url: String,
    signing_key: Arc<SigningKey>,
    directory: Arc<DirectoryKey>,
    pool: Pool<ConnectionManager<PgConnection>>,
}

impl CometBftDeviceKeyService {
    #[must_use]
    pub fn new(
        rpc_url: String,
        signing_key: Arc<SigningKey>,
        directory: Arc<DirectoryKey>,
        pool: Pool<ConnectionManager<PgConnection>>,
    ) -> Self {
        Self {
            http: Client::new(),
            rpc_url,
            signing_key,
            directory,
            pool,
        }
    }

    fn get_conn(
        &self,
    ) -> Result<r2d2::PooledConnection<ConnectionManager<PgConnection>>, AppError> {
        self.pool
            .get()
            .map_err(|e| AppError::PoolError(e.to_string()))
    }

    // Sent in the `user_hash` wire field.
    fn index_hex(&self, user: &User) -> String {
        hex::encode(self.directory.index_for(&user.username).index)
    }

    // Submits tx and returns immediately after check_tx with the tx hash.
    async fn broadcast_tx_sync(&self, tx: &KeyUploadTx) -> Result<String, AppError> {
        let tx_bytes = serde_json::to_vec(tx).map_err(|e| AppError::ValueError(e.to_string()))?;

        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "broadcast_tx_sync",
            params: BroadcastParams {
                tx: BASE64_STANDARD.encode(&tx_bytes),
            },
        };

        let res: JsonRpcResponse<BroadcastSyncResult> = self
            .http
            .post(&self.rpc_url)
            .json(&req)
            .send()
            .await
            .map_err(|e| AppError::ValueError(e.to_string()))?
            .json()
            .await
            .map_err(|e| AppError::ValueError(e.to_string()))?;

        let result = match res {
            JsonRpcResponse::Ok { result } => result,
            JsonRpcResponse::Err { error } => {
                return Err(AppError::ValueError(format!("rpc error: {error}")));
            }
        };

        if result.code != 0 {
            return Err(AppError::ValueError(format!(
                "check_tx rejected: {}",
                result.log
            )));
        }

        Ok(result.hash)
    }

    // Polls until the tx with the given hash is committed in a block.
    // CometBFT JSON-RPC expects `hash` as base64-encoded raw bytes.
    async fn wait_for_tx(&self, hash: &str) -> Result<(), AppError> {
        let hash_bytes = hex::decode(hash).map_err(|e| AppError::ValueError(e.to_string()))?;
        let hash_b64 = BASE64_STANDARD.encode(&hash_bytes);

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);

        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::ValueError(format!(
                    "timed out waiting for tx {hash} to commit"
                )));
            }

            let req = JsonRpcRequest {
                jsonrpc: "2.0",
                id: 1,
                method: "tx",
                params: TxQueryParams {
                    hash: hash_b64.clone(),
                    prove: false,
                },
            };

            let resp: serde_json::Value = self
                .http
                .post(&self.rpc_url)
                .json(&req)
                .send()
                .await
                .map_err(|e| AppError::ValueError(e.to_string()))?
                .json()
                .await
                .map_err(|e| AppError::ValueError(e.to_string()))?;

            if resp.get("error").is_some() {
                // Not yet committed — wait one poll interval and retry.
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                continue;
            }

            let code = resp["result"]["tx_result"]["code"].as_u64().unwrap_or(1);
            if code != 0 {
                let log = resp["result"]["tx_result"]["log"]
                    .as_str()
                    .unwrap_or("unknown error")
                    .to_owned();
                return Err(AppError::ValueError(format!(
                    "finalize_block rejected: {log}"
                )));
            }

            return Ok(());
        }
    }

    async fn abci_query(&self, path: &str, data: &[u8]) -> Result<Vec<u8>, AppError> {
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "abci_query",
            params: AbciQueryParams {
                path: path.to_owned(),
                data: hex::encode(data),
                prove: false,
            },
        };

        let res: JsonRpcResponse<AbciQueryResult> = self
            .http
            .post(&self.rpc_url)
            .json(&req)
            .send()
            .await
            .map_err(|e| AppError::ValueError(e.to_string()))?
            .json()
            .await
            .map_err(|e| AppError::ValueError(e.to_string()))?;

        let result = match res {
            JsonRpcResponse::Ok { result } => result,
            JsonRpcResponse::Err { error } => {
                return Err(AppError::ValueError(format!("rpc error: {error}")));
            }
        };

        if result.response.code != 0 {
            return Err(AppError::UserError(result.response.log));
        }

        let value = result
            .response
            .value
            .ok_or_else(|| AppError::ValueError("empty response value".into()))?;

        BASE64_STANDARD
            .decode(&value)
            .map_err(|e| AppError::InvalidB64(e.to_string()))
    }

    // Returns all committed txs matching the CometBFT event query, oldest first.
    // Each entry is (block_height, tx).
    async fn tx_search(&self, query: &str) -> Result<Vec<(u64, KeyUploadTx)>, AppError> {
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "tx_search",
            params: TxSearchParams {
                query: query.to_owned(),
                prove: false,
                page: "1".to_owned(),
                per_page: "100".to_owned(),
                order_by: "asc".to_owned(),
            },
        };

        let res: JsonRpcResponse<TxSearchResult> = self
            .http
            .post(&self.rpc_url)
            .json(&req)
            .send()
            .await
            .map_err(|e| AppError::ValueError(e.to_string()))?
            .json()
            .await
            .map_err(|e| AppError::ValueError(e.to_string()))?;

        let result = match res {
            JsonRpcResponse::Ok { result } => result,
            JsonRpcResponse::Err { error } => {
                return Err(AppError::ValueError(format!("rpc error: {error}")));
            }
        };

        result
            .txs
            .into_iter()
            .map(|info| {
                let height = info.height.parse::<u64>().unwrap_or(0);
                let bytes = BASE64_STANDARD
                    .decode(&info.tx)
                    .map_err(|e| AppError::InvalidB64(e.to_string()))?;
                let tx = serde_json::from_slice::<KeyUploadTx>(&bytes)
                    .map_err(|e| AppError::ValueError(e.to_string()))?;
                Ok((height, tx))
            })
            .collect()
    }
}

#[async_trait]
impl DeviceKeyService for CometBftDeviceKeyService {
    #[tracing::instrument(skip(self))]
    async fn new_device_for(&self, user: &User) -> Result<Device, AppError> {
        let mut conn = self.get_conn()?;
        let new_device = NewDevice {
            user_id: user.id,
            x25519: None,
            ed25519: None,
        };

        let device = diesel::insert_into(device::table)
            .values(&new_device)
            .returning(Device::as_returning())
            .get_result(&mut conn)?;

        Ok(device)
    }

    #[tracing::instrument(skip(self))]
    async fn get_device(&self, user: &User, device_id: DeviceId) -> Result<Device, AppError> {
        let user_hash_hex = self.index_hex(user);
        let query = format!("{user_hash_hex}:{device_id}");

        let value = self.abci_query("device", query.as_bytes()).await?;

        let keys: AbciDeviceKeys =
            serde_json::from_slice(&value).map_err(|e| AppError::ValueError(e.to_string()))?;

        Ok(Device {
            id: device_id,
            user_id: user.id,
            x25519: Some(keys.x25519.to_vec()),
            ed25519: Some(keys.ed25519.to_vec()),
        })
    }

    #[tracing::instrument(skip(self))]
    async fn get_all_devices(&self, user: &User) -> Result<Vec<Device>, AppError> {
        let user_hash_hex = self.index_hex(user);

        let value = match self.abci_query("devices", user_hash_hex.as_bytes()).await {
            Ok(v) => v,
            Err(AppError::UserError(_)) => return Ok(vec![]),
            Err(e) => return Err(e),
        };

        let map: HashMap<String, AbciDeviceKeys> =
            serde_json::from_slice(&value).map_err(|e| AppError::ValueError(e.to_string()))?;

        let devices = map
            .into_iter()
            .filter_map(|(id_str, keys)| {
                DeviceId::try_from(id_str.as_str()).ok().map(|id| Device {
                    id,
                    user_id: user.id,
                    x25519: Some(keys.x25519.to_vec()),
                    ed25519: Some(keys.ed25519.to_vec()),
                })
            })
            .collect();

        Ok(devices)
    }

    #[tracing::instrument(skip(self))]
    async fn set_device_keys(
        &self,
        user: &User,
        device_id: DeviceId,
        keys: InboundDevice,
    ) -> Result<Device, AppError> {
        let x25519 = Curve25519PublicKey::from_base64(&keys.x25519)
            .map_err(|e| AppError::InvalidKey(e.to_string()))?;
        let ed25519 = Ed25519PublicKey::from_base64(&keys.ed25519)
            .map_err(|e| AppError::InvalidKey(e.to_string()))?;

        let x25519_bytes: &[u8; 32] = x25519
            .as_bytes()
            .try_into()
            .map_err(|_| AppError::InvalidKeySize)?;
        let ed25519_bytes: &[u8; 32] = ed25519
            .as_bytes()
            .try_into()
            .map_err(|_| AppError::InvalidKeySize)?;

        let payload = KeyPayload {
            user_hash: self.index_hex(user),
            device_id: device_id.to_string(),
            x25519: BASE64_STANDARD_NO_PAD.encode(x25519_bytes),
            ed25519: BASE64_STANDARD_NO_PAD.encode(ed25519_bytes),
            signature: keys.signature.clone(),
            authorization: keys.authorization.clone(),
        };

        let msg = serde_json::to_vec(&payload).map_err(|e| AppError::ValueError(e.to_string()))?;
        let sig = self.signing_key.sign(&msg);

        let hash = self
            .broadcast_tx_sync(&KeyUploadTx {
                payload,
                signature: BASE64_STANDARD_NO_PAD.encode(sig.to_bytes()),
            })
            .await?;
        self.wait_for_tx(&hash).await?;

        // Store keys in DB as well
        let x25519_db = x25519.as_bytes().to_vec();
        let ed25519_db = ed25519.as_bytes().to_vec();
        let user = user.clone();
        let mut conn = self.get_conn()?;

        let device = tokio::task::spawn_blocking(move || {
            diesel::update(device::table)
                .filter(device::id.eq(device_id).and(device::user_id.eq(user.id)))
                .set((device::x25519.eq(x25519_db), device::ed25519.eq(ed25519_db)))
                .returning(Device::as_returning())
                .get_result(&mut conn)
        })
        .await??;

        Ok(device)
    }

    #[tracing::instrument(skip(self))]
    async fn get_valid_users(&self) -> Result<usize, AppError> {
        let mut conn = self.get_conn()?;
        let users = tokio::task::spawn_blocking(move || {
            user_table::table.select(User::as_select()).load(&mut conn)
        })
        .await??;

        let mut have_devices = 0_usize;
        for user in &users {
            if self
                .get_all_devices(user)
                .await
                .is_ok_and(|d| !d.is_empty())
            {
                have_devices += 1;
            }
        }

        Ok(have_devices)
    }

    #[tracing::instrument(skip(self))]
    async fn get_device_key_history(
        &self,
        user: &User,
        device_id: DeviceId,
    ) -> Result<Vec<HistoricalKey>, AppError> {
        let user_hash_hex = self.index_hex(user);
        let device_id_str = device_id.to_string();

        // key_add fires on first registration, key_update on each subsequent change.
        // Both are queried ascending by height so history is chronological.
        let mut history = Vec::new();
        for event_type in &["key_add", "key_update"] {
            let query = format!(
                "{event_type}.user_hash='{user_hash_hex}' AND {event_type}.device_id='{device_id_str}'"
            );
            for (chain_height, tx) in self.tx_search(&query).await? {
                let x25519 = BASE64_STANDARD_NO_PAD
                    .decode(&tx.payload.x25519)
                    .map_err(|e| AppError::InvalidB64(e.to_string()))?;
                let ed25519 = BASE64_STANDARD_NO_PAD
                    .decode(&tx.payload.ed25519)
                    .map_err(|e| AppError::InvalidB64(e.to_string()))?;
                let signature = BASE64_STANDARD_NO_PAD
                    .decode(&tx.payload.signature)
                    .map_err(|e| AppError::InvalidB64(e.to_string()))?;
                history.push(HistoricalKey {
                    device_id,
                    chain_height,
                    x25519,
                    ed25519,
                    signature,
                    authorization: tx.payload.authorization,
                });
            }
        }

        Ok(history)
    }
}
