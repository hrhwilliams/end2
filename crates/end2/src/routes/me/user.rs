use axum::{Json, extract::State, response::IntoResponse};
use serde::Deserialize;

use crate::{ApiError, AppState, NewNickname, User, UserResponse, WsEvent};

/// returns user struct if the user has the Session cookie set with a valid session token
#[tracing::instrument(skip(app_state))]
pub async fn me(State(app_state): State<AppState>, user: User) -> impl IntoResponse {
    Json(UserResponse::new(&app_state, user))
}

#[derive(Deserialize)]
pub struct Nickname {
    pub nickname: String,
}

#[tracing::instrument(skip(app_state))]
pub async fn change_nickname(
    State(app_state): State<AppState>,
    user: User,
    Json(Nickname { nickname }): Json<Nickname>,
) -> Result<impl IntoResponse, ApiError> {
    app_state.auth.change_nickname(&user, &nickname).await?;

    let users_to_notify = app_state.auth.get_known_users(&user).await?;
    for other in users_to_notify {
        app_state
            .relay
            .notify_user(
                &other,
                WsEvent::NicknameChanged(NewNickname {
                    user_id: user.id,
                    nickname: nickname.clone(),
                }),
            )
            .await;
    }

    Ok(Json(serde_json::json!({ "status": "success" })))
}
