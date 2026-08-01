use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing::info;

/// Aurelius project sync server — hub-and-spoke push/pull endpoint for
/// two-way project sync between Aurelius instances (see
/// specs/002-project-sync/). Routes are wired up in a later phase (US1);
/// this binary currently just validates its database and starts listening.
#[derive(Parser)]
#[command(
    name = "aurelius-sync-server",
    about = "Aurelius project sync server",
    version
)]
struct Cli {
    /// Port to listen on
    #[arg(short, long, default_value_t = 8181)]
    port: u16,

    /// Path to the server's own SQLite database file
    #[arg(long)]
    db: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    // Opens (creating if needed) and runs migrations against the server's
    // own SQLite file, shared schema with the client binaries.
    let conn = aurelius_core::db::open(&cli.db)?;
    drop(conn);

    // Placeholder router — POST /sync/push and GET /sync/pull land in
    // routes.rs during User Story 1 implementation.
    let app = axum::Router::new();

    let addr = format!("0.0.0.0:{}", cli.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!(%addr, db = %cli.db.display(), "aurelius-sync-server listening");
    axum::serve(listener, app).await?;

    Ok(())
}
