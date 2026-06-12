use axum::{extract::State, response::Json};
use serde::{Deserialize, Serialize};

use super::ApiError;
use crate::routes::admin::discover::{resolve_channel, resolve_manual, ResolvedMeta};
use crate::AppState;

#[derive(Serialize)]
pub struct ResolvedCandidate {
    pub url: String,
    pub title: String,
    pub duration_secs: i64,
    pub is_live: bool,
    pub source_kind: String,
}

impl From<ResolvedMeta> for ResolvedCandidate {
    fn from(m: ResolvedMeta) -> Self {
        ResolvedCandidate {
            url: m.url,
            title: m.title,
            duration_secs: m.duration_secs,
            is_live: m.is_live,
            source_kind: m.source_kind,
        }
    }
}

#[derive(Deserialize)]
pub struct ResolveRequest {
    pub url: String,
}

pub async fn resolve(
    State(_state): State<AppState>,
    Json(req): Json<ResolveRequest>,
) -> Result<Json<ResolvedCandidate>, ApiError> {
    let meta = resolve_manual(&req.url)
        .await
        .map_err(|_| ApiError::Validation("invalid or unresolvable URL".into()))?;
    Ok(Json(meta.into()))
}

pub async fn channel(
    State(_state): State<AppState>,
    Json(req): Json<ResolveRequest>,
) -> Result<Json<ResolvedCandidate>, ApiError> {
    let meta = resolve_channel(&req.url)
        .map_err(|_| ApiError::Validation("not a recognized YouTube channel URL".into()))?;
    Ok(Json(meta.into()))
}
