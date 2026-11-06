use axum::routing::{any, get, post};
use serde::Serialize;

use crate::{AppState, DirectoryIndex, User, handle_websocket};

mod auth;
mod channel;
mod device;
mod me;
mod user;
mod version;

/// A user plus the VRF proof that binds their directory index to their username.
#[derive(Serialize)]
pub struct UserResponse {
    #[serde(flatten)]
    user: User,
    directory: DirectoryIndex,
}

impl UserResponse {
    #[must_use]
    pub fn new(app_state: &AppState, user: User) -> Self {
        let directory = app_state.directory.index_for(&user.username);
        Self { user, directory }
    }
}

pub struct Api;

impl Api {
    pub fn router() -> axum::Router<AppState> {
        axum::Router::new()
            .route("/version", get(version::version))
            .route("/auth/register", post(auth::register))
            .route("/auth/login", post(auth::login))
            .route("/auth/logout", post(auth::logout))
            .route("/auth/discord", get(auth::get_discord_oauth_url))
            .route("/auth/google", get(auth::get_google_oauth_url))
            .route("/auth/redirect/google", get(auth::google_redirect))
            .route("/auth/redirect/discord", get(auth::discord_redirect))
            .route("/channel", post(channel::create_channel_with))
            .route("/channel/{channel_id}", get(channel::get_channel_info))
            .route(
                "/channel/{channel_id}/history",
                get(channel::get_channel_history),
            )
            .route("/channel/{channel_id}/msg", post(channel::send_message))
            .route("/me", get(me::me))
            .route("/me/nickname", post(me::change_nickname))
            .route("/me/channels", get(channel::get_all_channels))
            .route(
                "/me/device",
                post(device::new_device).put(device::upload_keys_me),
            )
            .route("/me/devices", get(device::get_devices))
            .route(
                "/me/device/{device_id}",
                get(device::get_device).put(device::upload_keys),
            )
            .route(
                "/me/device/{device_id}/otks",
                get(device::get_otks).post(device::upload_otks),
            )
            .route("/user/valid", get(user::get_valid_users))
            .route("/user/{user_id}", get(user::get_user_info))
            .route("/user/{user_id}/devices", get(device::get_user_devices))
            .route(
                "/user/{user_id}/device/{device_id}",
                get(device::get_user_device),
            )
            .route(
                "/user/{user_id}/device/{device_id}/history",
                get(device::get_user_device_key_history),
            )
            .route(
                "/user/{user_id}/device/{device_id}/otk",
                post(device::get_user_device_otk),
            )
            .route("/me/device/{device_id}/ws", any(handle_websocket))
    }
}
