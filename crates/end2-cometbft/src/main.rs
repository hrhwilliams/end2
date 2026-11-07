mod store;

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use base64::{Engine, prelude::BASE64_STANDARD_NO_PAD};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use jmt::{JellyfishMerkleTree, KeyHash, storage::TreeUpdateBatch};
use rocksdb::WriteBatch;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tendermint_abci::{Application, ServerBuilder};
use tendermint_proto::abci::{
    Event, EventAttribute, ExecTxResult, RequestCheckTx, RequestFinalizeBlock, RequestInfo,
    RequestQuery, ResponseCheckTx, ResponseCommit, ResponseFinalizeBlock, ResponseInfo,
    ResponseQuery,
};

use crate::store::{META_APP_HASH, META_HEIGHT, Store};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InboundAuthorization {
    pub authorizing_device_id: String,
    pub signature: String,
}

#[derive(Serialize, Deserialize)]
struct KeyPayload {
    user_hash: String,
    device_id: String,
    x25519: String,
    ed25519: String,
    signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorization: Option<InboundAuthorization>,
}

fn decode_key32(s: &str) -> Result<[u8; 32], &'static str> {
    BASE64_STANDARD_NO_PAD
        .decode(s)
        .map_err(|_| "key is not valid base64")?
        .try_into()
        .map_err(|_| "key must be 32 bytes")
}

fn decode_sig(s: &str) -> Result<Signature, &'static str> {
    let bytes: [u8; 64] = BASE64_STANDARD_NO_PAD
        .decode(s)
        .map_err(|_| "signature is not valid base64")?
        .try_into()
        .map_err(|_| "signature must be 64 bytes")?;
    Ok(Signature::from_bytes(&bytes))
}

#[derive(Serialize, Deserialize)]
struct KeyUploadTx {
    payload: KeyPayload,
    signature: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct DeviceKeys {
    x25519: [u8; 32],
    ed25519: [u8; 32],
}

// A tx that passed every stateless check: provider signature, encoding, device self-signature.
struct Upload {
    index: String,
    device_id: String,
    keys: DeviceKeys,
    self_signature: Signature,
    authorization: Option<(String, Signature)>,
}

// Pending state produced by finalize_block, written atomically in commit.
struct Pending {
    height: u64,
    app_hash: [u8; 32],
    // (rocksdb key, JSON(DeviceKeys)) — written into rocksdb on commit.
    writes: Vec<(Vec<u8>, Vec<u8>)>,
    tree_update: TreeUpdateBatch,
}

#[derive(Clone)]
struct KeyDirectoryApp {
    store: Arc<Store>,
    verifying_key: Arc<VerifyingKey>,
    pending: Arc<Mutex<Option<Pending>>>,
}

impl KeyDirectoryApp {
    fn new(verifying_key: VerifyingKey, store: Store) -> Self {
        Self {
            store: Arc::new(store),
            verifying_key: Arc::new(verifying_key),
            pending: Arc::new(Mutex::new(None)),
        }
    }
}

fn verify_tx(bytes: &[u8], key: &VerifyingKey) -> Result<Upload, &'static str> {
    let tx: KeyUploadTx = serde_json::from_slice(bytes).map_err(|_| "invalid JSON")?;

    let signature = decode_sig(&tx.signature)?;
    let msg = serde_json::to_vec(&tx.payload).map_err(|_| "failed to serialize payload")?;
    key.verify(&msg, &signature)
        .map_err(|_| "signature verification failed")?;

    let p = tx.payload;
    // Store keys are "<index>/<device_id>": a '/' in either would let one index write under another.
    let index_ok = !p.user_hash.is_empty()
        && p.user_hash
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if !index_ok || p.device_id.is_empty() || p.device_id.contains('/') {
        return Err("malformed index or device id");
    }

    let keys = DeviceKeys {
        x25519: decode_key32(&p.x25519)?,
        ed25519: decode_key32(&p.ed25519)?,
    };
    let self_signature = decode_sig(&p.signature)?;
    VerifyingKey::from_bytes(&keys.ed25519)
        .map_err(|_| "invalid ed25519 key")?
        .verify_strict(&[keys.x25519, keys.ed25519].concat(), &self_signature)
        .map_err(|_| "device self-signature verification failed")?;

    let authorization = match p.authorization {
        Some(a) => Some((a.authorizing_device_id, decode_sig(&a.signature)?)),
        None => None,
    };

    Ok(Upload {
        index: p.user_hash,
        device_id: p.device_id,
        keys,
        self_signature,
        authorization,
    })
}

/// Decides whether `upload` may be written, given every device already under its index
/// (committed state plus earlier txs in the same block). Returns the event type.
fn admit(
    upload: &Upload,
    existing: &BTreeMap<String, DeviceKeys>,
) -> Result<&'static str, &'static str> {
    if existing.is_empty() {
        return Ok("key_add");
    }

    let (authorizing_id, signature) = upload
        .authorization
        .as_ref()
        .ok_or("index already has a root key; authorization required")?;
    let is_update = existing.contains_key(&upload.device_id);
    if is_update && *authorizing_id != upload.device_id {
        return Err("key update must be authorized by the same device's current key");
    }
    let authorizing = existing
        .get(authorizing_id)
        .ok_or("authorizing device is not registered under this index")?;
    VerifyingKey::from_bytes(&authorizing.ed25519)
        .map_err(|_| "authorizing device has an invalid ed25519 key")?
        .verify_strict(&upload.self_signature.to_bytes(), signature)
        .map_err(|_| "authorization signature verification failed")?;

    Ok(if is_update { "key_update" } else { "key_add" })
}

fn devices_under(
    store: &Store,
    overlay: &BTreeMap<Vec<u8>, Vec<u8>>,
    index: &str,
) -> BTreeMap<String, DeviceKeys> {
    let prefix = Store::device_prefix(index);
    let pending = overlay
        .range(prefix.clone()..)
        .take_while(|(k, _)| k.starts_with(&prefix))
        .map(|(k, v)| {
            let id = std::str::from_utf8(&k[prefix.len()..]).expect("utf8 device id in overlay");
            (id.to_owned(), v.clone())
        });
    // Overlay entries come last so they replace committed ones in the collected map.
    store
        .iter_user_devices(index)
        .into_iter()
        .chain(pending)
        .map(|(id, v)| {
            (
                id,
                serde_json::from_slice(&v).expect("stored DeviceKeys json"),
            )
        })
        .collect()
}

fn check_tx_err(log: &str) -> ResponseCheckTx {
    ResponseCheckTx {
        code: 1,
        log: log.to_owned(),
        ..Default::default()
    }
}

fn err_query(msg: &str) -> ResponseQuery {
    ResponseQuery {
        code: 1,
        log: msg.to_owned(),
        ..Default::default()
    }
}

impl Application for KeyDirectoryApp {
    fn info(&self, _req: RequestInfo) -> ResponseInfo {
        let height = self.store.last_height();
        let app_hash = self.store.last_app_hash();
        ResponseInfo {
            data: "end2-cometbft".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            app_version: 1,
            last_block_height: height as i64,
            last_block_app_hash: app_hash.into(),
        }
    }

    fn check_tx(&self, req: RequestCheckTx) -> ResponseCheckTx {
        match verify_tx(&req.tx, &self.verifying_key) {
            Ok(_) => ResponseCheckTx::default(),
            Err(msg) => check_tx_err(msg),
        }
    }

    fn finalize_block(&self, req: RequestFinalizeBlock) -> ResponseFinalizeBlock {
        let mut tx_results: Vec<ExecTxResult> = Vec::with_capacity(req.txs.len());

        // Overlay of this block's writes keyed by rocksdb key, for hash computation
        // and so admission of later txs in the block sees earlier ones.
        let mut overlay: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        let mut tree_updates: Vec<(KeyHash, Option<Vec<u8>>)> = Vec::with_capacity(req.txs.len());

        for raw in &req.txs {
            let admitted = verify_tx(raw, &self.verifying_key).and_then(|upload| {
                let existing = devices_under(&self.store, &overlay, &upload.index);
                admit(&upload, &existing).map(|event_type| (upload, event_type))
            });
            let (upload, event_type) = match admitted {
                Ok(v) => v,
                Err(msg) => {
                    tx_results.push(ExecTxResult {
                        code: 1,
                        log: msg.to_owned(),
                        ..Default::default()
                    });
                    continue;
                }
            };

            let Upload {
                index: user_hash_hex,
                device_id,
                keys,
                ..
            } = upload;
            let rk = Store::device_key(&user_hash_hex, &device_id);

            let value_json = serde_json::to_vec(&keys).expect("DeviceKeys json");
            overlay.insert(rk.clone(), value_json.clone());

            let key_hash = KeyHash::with::<Sha256>(&rk);
            let val_hash = Sha256::digest(&value_json).to_vec();
            tree_updates.push((key_hash, Some(val_hash)));

            tx_results.push(ExecTxResult {
                events: vec![Event {
                    r#type: event_type.to_owned(),
                    attributes: vec![
                        EventAttribute {
                            key: "user_hash".to_owned(),
                            value: user_hash_hex,
                            index: true,
                        },
                        EventAttribute {
                            key: "device_id".to_owned(),
                            value: device_id,
                            index: true,
                        },
                    ],
                }],
                ..Default::default()
            });
        }

        let version = req.height as u64;

        let tree = JellyfishMerkleTree::<_, Sha256>::new(self.store.as_ref());
        let (root_hash, tree_update) = tree
            .put_value_set(tree_updates, version)
            .expect("JMT put_value_set failed");

        let writes: Vec<(Vec<u8>, Vec<u8>)> = overlay.into_iter().collect();
        *self.pending.lock().expect("lock pending") = Some(Pending {
            height: version,
            app_hash: root_hash.0,
            writes,
            tree_update,
        });

        ResponseFinalizeBlock {
            tx_results,
            app_hash: root_hash.0.to_vec().into(),
            ..Default::default()
        }
    }

    fn commit(&self) -> ResponseCommit {
        let pending = self.pending.lock().unwrap().take();
        if let Some(p) = pending {
            let mut batch = WriteBatch::default();

            for (k, v) in p.writes {
                batch.put_cf(self.store.cf_device(), k, v);
            }

            self.store.write_tree_update(&mut batch, p.tree_update);

            batch.put(META_HEIGHT, p.height.to_le_bytes());
            batch.put(META_APP_HASH, p.app_hash);

            let mut wo = rocksdb::WriteOptions::default();
            wo.set_sync(true);
            self.store
                .db
                .write_opt(batch, &wo)
                .expect("rocksdb commit write failed");
        }
        ResponseCommit::default()
    }

    /// Supported query paths. Store key is `hex(user_hash)`, a 64-char hex string.
    ///
    /// `"device"` - fetch one device's keys.
    /// `data` = `"<hex_user_hash>:<device_id>"` as UTF-8, then hex-encoded for the RPC.
    /// Returns `DeviceKeys` as JSON in `response.value` (RPC base64-encodes it).
    ///
    /// `"devices"` - fetch all devices for a user.
    /// `data` = `"<hex_user_hash>"` as UTF-8, then hex-encoded for the RPC.
    /// Returns `HashMap<device_id, DeviceKeys>` as JSON.
    fn query(&self, req: RequestQuery) -> ResponseQuery {
        match req.path.as_str() {
            "device" => {
                let raw = String::from_utf8_lossy(&req.data);
                let Some((user_hash_hex, device_id)) = raw.split_once(':') else {
                    return err_query("data must be '<hex_user_hash>:<device_id>'");
                };
                match self.store.get_device(user_hash_hex, device_id) {
                    Some(bytes) => ResponseQuery {
                        value: bytes.into(),
                        ..Default::default()
                    },
                    None => err_query(&format!(
                        "no device '{device_id}' for hash '{user_hash_hex}'"
                    )),
                }
            }

            "devices" => {
                let user_hash_hex = String::from_utf8_lossy(&req.data);
                let v = self.store.iter_user_devices(user_hash_hex.as_ref());
                if v.is_empty() {
                    return err_query(&format!("no devices for hash '{user_hash_hex}'"));
                }
                let mut map = serde_json::Map::with_capacity(v.len());
                for (id, bytes) in v {
                    let val: serde_json::Value =
                        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
                    map.insert(id, val);
                }
                ResponseQuery {
                    value: serde_json::to_vec(&serde_json::Value::Object(map))
                        .unwrap_or_default()
                        .into(),
                    ..Default::default()
                }
            }

            other => err_query(&format!(
                "unknown path '{other}': use 'device' or 'devices'"
            )),
        }
    }
}

fn main() -> Result<(), tendermint_abci::Error> {
    dotenvy::dotenv().ok();

    let pubkey_b64 = std::env::var("ABCI_SERVER_PUBKEY").expect("ABCI_SERVER_PUBKEY must be set");
    let pubkey_bytes = BASE64_STANDARD_NO_PAD
        .decode(pubkey_b64)
        .expect("ABCI_SERVER_PUBKEY is not valid base64");
    let pubkey_bytes: [u8; 32] = pubkey_bytes
        .try_into()
        .expect("ABCI_SERVER_PUBKEY must be 32 bytes");
    let verifying_key =
        VerifyingKey::from_bytes(&pubkey_bytes).expect("invalid ed25519 public key");

    let port = std::env::var("ABCI_PORT").unwrap_or_else(|_| "26658".into());
    let db_path = std::env::var("ABCI_DB_PATH").unwrap_or_else(|_| "./abci-data".into());

    let store = Store::open(&db_path).expect("failed to open rocksdb");
    let app = KeyDirectoryApp::new(verifying_key, store);

    ServerBuilder::default()
        .bind(format!("0.0.0.0:{port}"), app)
        .expect("failed to bind ABCI server")
        .listen()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const INDEX: &str = "ab12";
    const NOT_AUTHORIZED: &str = "index already has a root key; authorization required";

    struct Dev {
        sk: SigningKey,
        x25519: [u8; 32],
    }

    impl Dev {
        fn new(seed: u8) -> Self {
            Self {
                sk: SigningKey::from_bytes(&[seed; 32]),
                x25519: [seed.wrapping_add(100); 32],
            }
        }

        fn ed25519(&self) -> [u8; 32] {
            self.sk.verifying_key().to_bytes()
        }

        fn self_sig(&self) -> Signature {
            self.sk.sign(&[self.x25519, self.ed25519()].concat())
        }
    }

    fn provider() -> SigningKey {
        SigningKey::from_bytes(&[1; 32])
    }

    fn upload_signed(
        index: &str,
        device_id: &str,
        dev: &Dev,
        self_sig: Signature,
        authorized_by: Option<(&str, &Dev)>,
    ) -> Vec<u8> {
        let payload = KeyPayload {
            user_hash: index.to_owned(),
            device_id: device_id.to_owned(),
            x25519: BASE64_STANDARD_NO_PAD.encode(dev.x25519),
            ed25519: BASE64_STANDARD_NO_PAD.encode(dev.ed25519()),
            signature: BASE64_STANDARD_NO_PAD.encode(self_sig.to_bytes()),
            authorization: authorized_by.map(|(id, by)| InboundAuthorization {
                authorizing_device_id: id.to_owned(),
                signature: BASE64_STANDARD_NO_PAD
                    .encode(by.sk.sign(&self_sig.to_bytes()).to_bytes()),
            }),
        };
        let signature = provider().sign(&serde_json::to_vec(&payload).expect("payload json"));
        serde_json::to_vec(&KeyUploadTx {
            payload,
            signature: BASE64_STANDARD_NO_PAD.encode(signature.to_bytes()),
        })
        .expect("tx json")
    }

    fn upload(
        index: &str,
        device_id: &str,
        dev: &Dev,
        authorized_by: Option<(&str, &Dev)>,
    ) -> Vec<u8> {
        upload_signed(index, device_id, dev, dev.self_sig(), authorized_by)
    }

    struct Chain {
        app: KeyDirectoryApp,
        height: i64,
    }

    impl Chain {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("end2-cometbft-test-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            let store = Store::open(dir.to_str().expect("utf8 temp path")).expect("open store");
            Self {
                app: KeyDirectoryApp::new(provider().verifying_key(), store),
                height: 0,
            }
        }

        fn block(&mut self, txs: Vec<Vec<u8>>) -> Vec<ExecTxResult> {
            self.height += 1;
            let res = self.app.finalize_block(RequestFinalizeBlock {
                txs: txs.into_iter().map(Into::into).collect(),
                height: self.height,
                ..Default::default()
            });
            self.app.commit();
            res.tx_results
        }

        fn check_tx(&self, tx: &[u8]) -> u32 {
            self.app
                .check_tx(RequestCheckTx {
                    tx: tx.to_vec().into(),
                    ..Default::default()
                })
                .code
        }

        fn ed25519_by_device(&self, index: &str) -> BTreeMap<String, [u8; 32]> {
            let res = self.app.query(RequestQuery {
                path: "devices".into(),
                data: index.as_bytes().to_vec().into(),
                ..Default::default()
            });
            if res.code != 0 {
                return BTreeMap::new();
            }
            serde_json::from_slice::<BTreeMap<String, DeviceKeys>>(&res.value)
                .expect("devices json")
                .into_iter()
                .map(|(id, keys)| (id, keys.ed25519))
                .collect()
        }
    }

    fn event_type(res: &ExecTxResult) -> &str {
        &res.events[0].r#type
    }

    fn expected(entries: &[(&str, &Dev)]) -> BTreeMap<String, [u8; 32]> {
        entries
            .iter()
            .map(|(id, dev)| ((*id).to_owned(), dev.ed25519()))
            .collect()
    }

    #[test]
    fn root_key_on_empty_index_is_accepted() {
        let mut chain = Chain::new("root");
        let alice = Dev::new(10);

        let res = chain.block(vec![upload(INDEX, "phone", &alice, None)]);

        assert_eq!(res[0].code, 0, "{}", res[0].log);
        assert_eq!(event_type(&res[0]), "key_add");
        assert_eq!(
            chain.ed25519_by_device(INDEX),
            expected(&[("phone", &alice)])
        );
    }

    #[test]
    fn provider_cannot_register_second_unauthorized_root_under_claimed_index() {
        let mut chain = Chain::new("second-root");
        let alice = Dev::new(10);
        let attacker = Dev::new(66);
        chain.block(vec![upload(INDEX, "phone", &alice, None)]);

        let res = chain.block(vec![upload(INDEX, "attacker", &attacker, None)]);

        assert_ne!(res[0].code, 0);
        assert_eq!(res[0].log, NOT_AUTHORIZED);
        assert_eq!(
            chain.ed25519_by_device(INDEX),
            expected(&[("phone", &alice)])
        );
    }

    #[test]
    fn second_device_authorized_by_root_is_accepted() {
        let mut chain = Chain::new("second-device");
        let phone = Dev::new(10);
        let laptop = Dev::new(11);
        chain.block(vec![upload(INDEX, "phone", &phone, None)]);

        let res = chain.block(vec![upload(
            INDEX,
            "laptop",
            &laptop,
            Some(("phone", &phone)),
        )]);

        assert_eq!(res[0].code, 0, "{}", res[0].log);
        assert_eq!(event_type(&res[0]), "key_add");
        assert_eq!(
            chain.ed25519_by_device(INDEX),
            expected(&[("laptop", &laptop), ("phone", &phone)])
        );
    }

    #[test]
    fn key_update_without_valid_authorization_is_rejected() {
        let mut chain = Chain::new("unauthorized-update");
        let phone = Dev::new(10);
        let attacker = Dev::new(66);
        chain.block(vec![upload(INDEX, "phone", &phone, None)]);

        let res = chain.block(vec![
            upload(INDEX, "phone", &attacker, None),
            upload(INDEX, "phone", &attacker, Some(("phone", &attacker))),
        ]);

        assert_eq!(res[0].log, NOT_AUTHORIZED);
        assert_ne!(
            res[1].code, 0,
            "authorization signed by a key not on the index"
        );
        assert_eq!(
            chain.ed25519_by_device(INDEX),
            expected(&[("phone", &phone)])
        );
    }

    #[test]
    fn rotation_authorized_by_the_device_itself_is_accepted() {
        let mut chain = Chain::new("rotation");
        let old = Dev::new(10);
        let new = Dev::new(12);
        chain.block(vec![upload(INDEX, "phone", &old, None)]);

        let res = chain.block(vec![upload(INDEX, "phone", &new, Some(("phone", &old)))]);

        assert_eq!(res[0].code, 0, "{}", res[0].log);
        assert_eq!(event_type(&res[0]), "key_update");
        assert_eq!(chain.ed25519_by_device(INDEX), expected(&[("phone", &new)]));
    }

    #[test]
    fn rotation_authorized_by_a_different_device_under_the_same_index_is_rejected() {
        let mut chain = Chain::new("rotation-by-other");
        let phone = Dev::new(10);
        let laptop = Dev::new(11);
        let new = Dev::new(12);
        chain.block(vec![upload(INDEX, "phone", &phone, None)]);
        chain.block(vec![upload(
            INDEX,
            "laptop",
            &laptop,
            Some(("phone", &phone)),
        )]);

        let res = chain.block(vec![upload(
            INDEX,
            "phone",
            &new,
            Some(("laptop", &laptop)),
        )]);

        assert_eq!(
            res[0].log,
            "key update must be authorized by the same device's current key"
        );
        assert_eq!(
            chain.ed25519_by_device(INDEX),
            expected(&[("laptop", &laptop), ("phone", &phone)])
        );
    }

    #[test]
    fn bad_self_signature_is_rejected_by_check_tx_and_finalize_block() {
        let mut chain = Chain::new("bad-self-sig");
        let alice = Dev::new(10);
        let other = Dev::new(66);
        let forged = upload_signed(INDEX, "phone", &alice, other.self_sig(), None);

        assert_eq!(chain.check_tx(&upload(INDEX, "phone", &alice, None)), 0);
        assert_ne!(chain.check_tx(&forged), 0);

        let res = chain.block(vec![forged]);

        assert_ne!(res[0].code, 0);
        assert!(chain.ed25519_by_device(INDEX).is_empty());
    }

    #[test]
    fn second_unauthorized_root_in_same_block_is_rejected() {
        let mut chain = Chain::new("same-block");
        let alice = Dev::new(10);
        let attacker = Dev::new(66);

        let res = chain.block(vec![
            upload(INDEX, "phone", &alice, None),
            upload(INDEX, "attacker", &attacker, None),
        ]);

        assert_eq!(res[0].code, 0, "{}", res[0].log);
        assert_eq!(res[1].log, NOT_AUTHORIZED);
        assert_eq!(
            chain.ed25519_by_device(INDEX),
            expected(&[("phone", &alice)])
        );
    }

    #[test]
    fn index_containing_store_separator_is_rejected() {
        let mut chain = Chain::new("separator");
        let alice = Dev::new(10);
        let attacker = Dev::new(66);
        chain.block(vec![upload(INDEX, "phone", &alice, None)]);

        let res = chain.block(vec![upload(&format!("{INDEX}/x"), "dev", &attacker, None)]);

        assert_ne!(res[0].code, 0);
        assert_eq!(
            chain.ed25519_by_device(INDEX),
            expected(&[("phone", &alice)])
        );
    }
}
