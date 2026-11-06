use axum::{Json, extract::State, response::IntoResponse};

use crate::{ApiError, AppState, InboundUser, UserResponse, WebSession};

#[tracing::instrument(skip(app_state, new_user), fields(user = %new_user.username))]
pub async fn register(
    State(app_state): State<AppState>,
    web_session: Option<WebSession>,
    Json(new_user): Json<InboundUser>,
) -> Result<impl IntoResponse, ApiError> {
    let user = app_state.auth.register_user(new_user).await.map_err(|e| {
        tracing::error!(?e, "failed to register user");
        e
    })?;

    if let Some(web_session) = web_session {
        app_state
            .web_sessions
            .insert_into_session(web_session, "user_id".to_string(), user.id)
            .await
            .map_err(|e| {
                tracing::error!(?e, "failed to insert into session");
                e
            })?;
    }

    Ok(Json(UserResponse::new(&app_state, user)))
}
