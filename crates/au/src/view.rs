use anyhow::Result;
use aurelius_core::{db, graph};
use axum::{
    extract::Query,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use rust_embed::Embed;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Embed)]
#[folder = "../../ui/dist/"]
struct UiAssets;

fn db_path() -> PathBuf {
    let base = dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("aurelius");
    base.join("aurelius.db")
}

pub async fn serve(port: u16, no_open: bool) -> Result<()> {
    let app = Router::new()
        .route("/api/graph", get(api_graph))
        .route("/api/search", get(api_search))
        .fallback(get(serve_static));

    let addr = format!("127.0.0.1:{port}");
    let url = format!("http://localhost:{port}");

    println!("Aurelius UI starting at {url}");

    if !no_open {
        let url_clone = url.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let _ = open::that(&url_clone);
        });
    }

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn api_graph() -> Result<Json<serde_json::Value>, StatusCode> {
    let result = tokio::task::spawn_blocking(|| {
        let conn = db::open(&db_path()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let nodes = graph::get_all_nodes(&conn).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let edges = graph::get_all_edges(&conn).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok::<_, StatusCode>(serde_json::json!({ "nodes": nodes, "edges": edges }))
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;

    Ok(Json(result))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

async fn api_search(
    Query(params): Query<SearchQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let result = tokio::task::spawn_blocking(move || {
        let conn = db::open(&db_path()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let nodes =
            graph::search(&conn, &params.q, 50).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let edges = graph::get_all_edges(&conn).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let node_ids: std::collections::HashSet<String> =
            nodes.iter().map(|n| n.id.to_string()).collect();
        let filtered_edges: Vec<_> = edges
            .into_iter()
            .filter(|e| {
                node_ids.contains(&e.from_id.to_string()) && node_ids.contains(&e.to_id.to_string())
            })
            .collect();
        Ok::<_, StatusCode>(serde_json::json!({ "nodes": nodes, "edges": filtered_edges }))
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;

    Ok(Json(result))
}

// Serve the built React app embedded in the binary
async fn serve_static(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = UiAssets::get(path) {
        let mime = mime_from_path(path);
        return ([(header::CONTENT_TYPE, mime)], file.data.into_owned()).into_response();
    }

    // SPA fallback: serve index.html for client-side routing
    if let Some(index) = UiAssets::get("index.html") {
        return (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            index.data.into_owned(),
        )
            .into_response();
    }

    (
        StatusCode::NOT_FOUND,
        "UI assets not found. Rebuild with: cd ui && npm run build && cargo build",
    )
        .into_response()
}

fn mime_from_path(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}
