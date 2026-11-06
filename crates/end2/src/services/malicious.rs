use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine, prelude::BASE64_STANDARD_NO_PAD};
use diesel::{PgConnection, r2d2::ConnectionManager};
use ed25519_dalek::SigningKey;
use r2d2::Pool;

use crate::{
    AppError, CometBftDeviceKeyService, Device, DeviceId, DeviceKeyService, DirectoryKey,
    HistoricalKey, InboundDevice, User,
};

// Attacker-controlled keys distributed in place of the real ones when a
// client looks up a device. History queries still hit the chain unchanged,
// so a client comparing the distributed key against the on-chain history
// will see a mismatch.
const ATTACKER_X25519_B64: &str = "IeyxjOtLNBl9EZe9f0T/i9LBthJp1HicM1Sjd0/Lw3w";
const ATTACKER_ED25519_B64: &str = "8dvhUrR02tiWTf5IKQTD2l0pXPm6Ja/+Bzbnvrub468";

#[derive(Clone)]
pub struct MaliciousDeviceKeyService {
    inner: CometBftDeviceKeyService,
}

impl MaliciousDeviceKeyService {
    #[must_use]
    pub fn new(
        rpc_url: String,
        signing_key: Arc<SigningKey>,
        directory: Arc<DirectoryKey>,
        pool: Pool<ConnectionManager<PgConnection>>,
    ) -> Self {
        Self {
            inner: CometBftDeviceKeyService::new(rpc_url, signing_key, directory, pool),
        }
    }

    fn attacker_device(user_id: crate::UserId, device_id: DeviceId) -> Result<Device, AppError> {
        let x25519 = BASE64_STANDARD_NO_PAD
            .decode(ATTACKER_X25519_B64)
            .map_err(|e| AppError::InvalidB64(e.to_string()))?;
        let ed25519 = BASE64_STANDARD_NO_PAD
            .decode(ATTACKER_ED25519_B64)
            .map_err(|e| AppError::InvalidB64(e.to_string()))?;
        Ok(Device {
            id: device_id,
            user_id,
            x25519: Some(x25519),
            ed25519: Some(ed25519),
        })
    }
}

#[async_trait]
impl DeviceKeyService for MaliciousDeviceKeyService {
    async fn new_device_for(&self, user: &User) -> Result<Device, AppError> {
        self.inner.new_device_for(user).await
    }

    async fn get_device(&self, user: &User, device_id: DeviceId) -> Result<Device, AppError> {
        Self::attacker_device(user.id, device_id)
    }

    async fn get_all_devices(&self, user: &User) -> Result<Vec<Device>, AppError> {
        let devices = self.inner.get_all_devices(user).await?;
        devices
            .into_iter()
            .map(|d| Self::attacker_device(user.id, d.id))
            .collect()
    }

    async fn set_device_keys(
        &self,
        user: &User,
        device_id: DeviceId,
        keys: InboundDevice,
    ) -> Result<Device, AppError> {
        self.inner.set_device_keys(user, device_id, keys).await
    }

    async fn get_valid_users(&self) -> Result<usize, AppError> {
        self.inner.get_valid_users().await
    }

    async fn get_device_key_history(
        &self,
        user: &User,
        device_id: DeviceId,
    ) -> Result<Vec<HistoricalKey>, AppError> {
        self.inner.get_device_key_history(user, device_id).await
    }
}
