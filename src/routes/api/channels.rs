use axum::{extract::State, response::Json};

use super::{internal, ApiError};
use crate::{model::channel, AppState};

pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<channel::Channel>>, ApiError> {
    let channels = channel::list(&state.pool).await.map_err(internal)?;
    Ok(Json(channels))
}
