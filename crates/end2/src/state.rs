use std::sync::Arc;

use diesel::{PgConnection, r2d2::ConnectionManager};
use ed25519_dalek::SigningKey;
use r2d2::Pool;
use tokio::sync::broadcast;

use crate::{
    AppError, CookieWebSessionService, DirectoryKey,
    services::{AuthService, DeviceKeyService, MessageRelayService, OtkService},
};

#[derive(Clone)]
pub enum AppEvent {}

#[derive(Clone)]
pub struct AppState {
    pub auth: Arc<dyn AuthService>,
    pub device_keys: Arc<dyn DeviceKeyService>,
    pub otks: Arc<dyn OtkService>,
    pub relay: Arc<dyn MessageRelayService>,
    pub web_sessions: CookieWebSessionService,
    pub signing_key: Arc<SigningKey>,
    pub directory: Arc<DirectoryKey>,
    pool: Pool<ConnectionManager<PgConnection>>,
    /// Sends `AppEvent`s to subscribers
    broadcaster: broadcast::Sender<AppEvent>,
}

impl AppState {
    #[must_use]
    pub fn new(
        auth: Arc<dyn AuthService>,
        device_keys: Arc<dyn DeviceKeyService>,
        otks: Arc<dyn OtkService>,
        relay: Arc<dyn MessageRelayService>,
        pool: Pool<ConnectionManager<PgConnection>>,
        signing_key: Arc<SigningKey>,
        directory: Arc<DirectoryKey>,
    ) -> Self {
        let (broadcaster, _) = broadcast::channel(256);

        Self {
            auth,
            device_keys,
            otks,
            relay,
            web_sessions: CookieWebSessionService::new(pool.clone()),
            signing_key,
            directory,
            pool,
            broadcaster,
        }
    }

    pub(crate) fn get_conn(
        &self,
    ) -> Result<r2d2::PooledConnection<ConnectionManager<PgConnection>>, AppError> {
        self.pool
            .get()
            .map_err(|e| AppError::PoolError(e.to_string()))
    }
}
