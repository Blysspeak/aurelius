//! Library surface for `aurelius-sync-server`, split out from `main.rs` so
//! integration tests (`tests/`) can build the real `axum::Router` and drive
//! it over an actual bound TCP port, per
//! `specs/002-project-sync/contracts/sync-api.md`.

pub mod auth;
pub mod routes;

use std::path::PathBuf;

/// Shared state handed to every route/extractor: where the server's SQLite
/// file lives, and the admin credential that guards `/sync/grants*`.
#[derive(Clone)]
pub struct AppState {
    pub db_path: PathBuf,
    pub admin_token: String,
}

/// Builds the full `/sync`-mounted router for a given [`AppState`].
pub fn build_router(state: AppState) -> axum::Router {
    axum::Router::new()
        .nest("/sync", routes::router())
        .with_state(state)
}
