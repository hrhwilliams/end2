use std::sync::Arc;

use async_trait::async_trait;
use diesel::{PgConnection, r2d2::ConnectionManager};
use ed25519_dalek::SigningKey;
use r2d2::Pool;

use crate::{
    AppError, CometBftDeviceKeyService, Device, DeviceId, DeviceKeyService, DirectoryKey,
    HistoricalKey, InboundDevice, User,
};

// Attacker keys injected as a forged secondary device. The self-signature
// is valid for the attacker's keypair, but no `authorization` is supplied.
// The CometBFT validators reject the tx because the victim's index already
// holds a root key, so the forged device never reaches the chain.
const ATTACKER_X25519_B64: &str = "IeyxjOtLNBl9EZe9f0T/i9LBthJp1HicM1Sjd0/Lw3w";
const ATTACKER_ED25519_B64: &str = "8dvhUrR02tiWTf5IKQTD2l0pXPm6Ja/+Bzbnvrub468";
const ATTACKER_SELF_SIG_B64: &str =
    "heo2mtH9lguiG0EqHaR6FPeUvgKmNVoHKuEUkVFS88a9SMB7vyr2RbUZLNxQ2wiwUs+hkus7qGyLbUCizDLaBQ";

#[derive(Clone)]
pub struct ForgingDeviceKeyService {
    inner: CometBftDeviceKeyService,
}

impl ForgingDeviceKeyService {
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

    // Submits a forged tx for `user` adding a new device with attacker
    // keys and no authorization. Skips the normal route-level validation
    // because we are simulating the server itself acting maliciously.
    async fn inject_forged_device(&self, user: &User) -> Result<(), AppError> {
        let phantom = self.inner.new_device_for(user).await?;

        let inbound = InboundDevice {
            device_id: Some(phantom.id),
            x25519: ATTACKER_X25519_B64.to_owned(),
            ed25519: ATTACKER_ED25519_B64.to_owned(),
            signature: ATTACKER_SELF_SIG_B64.to_owned(),
            authorization: None,
        };

        self.inner
            .set_device_keys(user, phantom.id, inbound)
            .await?;

        Ok(())
    }
}

#[async_trait]
impl DeviceKeyService for ForgingDeviceKeyService {
    async fn new_device_for(&self, user: &User) -> Result<Device, AppError> {
        self.inner.new_device_for(user).await
    }

    async fn get_device(&self, user: &User, device_id: DeviceId) -> Result<Device, AppError> {
        self.inner.get_device(user, device_id).await
    }

    async fn get_all_devices(&self, user: &User) -> Result<Vec<Device>, AppError> {
        self.inner.get_all_devices(user).await
    }

    async fn set_device_keys(
        &self,
        user: &User,
        device_id: DeviceId,
        keys: InboundDevice,
    ) -> Result<Device, AppError> {
        // Honest path first so the victim's own request appears to succeed.
        let device = self.inner.set_device_keys(user, device_id, keys).await?;

        // Then quietly enroll an extra attacker-controlled device for the
        // victim. Failure is logged but not propagated — the victim's
        // request must still look successful.
        if let Err(e) = self.inject_forged_device(user).await {
            tracing::warn!(error = %e, "forged device injection failed");
        }

        Ok(device)
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
