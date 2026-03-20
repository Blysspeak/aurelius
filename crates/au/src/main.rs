mod commands;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "au", about = "Aurelius — personal knowledge graph", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize Aurelius in current environment
    Init,
    /// Add a knowledge node manually
    Note {
        /// The note content (decision, observation, etc.)
        text: String,
        /// Node type: decision, concept, problem, solution
        #[arg(short, long, default_value = "decision")]
        r#type: String,
        /// Label (short name). Defaults to first 50 chars of text.
        #[arg(short, long)]
        label: Option<String>,
    },
    /// Show knowledge graph context around a topic
    Context {
        topic: String,
        /// Graph traversal depth
        #[arg(short, long, default_value = "2")]
        depth: u32,
    },
    /// Search the knowledge graph
    Search { query: String },
    /// Sync from all connectors (git, beads, timeforged, beacon)
    Sync,
    /// Export full graph to JSON
    Export,
    /// Start MCP server (used by Claude Code)
    Mcp,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init => commands::init().await,
        Commands::Note { text, r#type, label } => commands::note(&text, &r#type, label).await,
        Commands::Context { topic, depth } => commands::context(&topic, depth).await,
        Commands::Search { query } => commands::search(&query).await,
        Commands::Sync => commands::sync().await,
        Commands::Export => commands::export().await,
        Commands::Mcp => commands::mcp().await,
    }
}
