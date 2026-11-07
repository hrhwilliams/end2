use axum::{Json, extract::State, response::IntoResponse};
use secrecy::SecretString;
use serde::Deserialize;

use crate::{ApiError, AppState, UserResponse, WebSession};

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: SecretString,
}

#[tracing::instrument(skip(app_state, password))]
pub async fn login(
    State(app_state): State<AppState>,
    web_session: WebSession,
    Json(LoginRequest { username, password }): Json<LoginRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let user = app_state.auth.login(&username, password).await?;
    app_state
        .web_sessions
        .insert_into_session(web_session, "user_id".to_string(), user.id)
        .await?;
    Ok(Json(UserResponse::new(&app_state, user)))
}
