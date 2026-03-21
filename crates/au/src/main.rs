mod commands;
mod view;

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
        /// Link to a project node (find or create by name)
        #[arg(short, long)]
        project: Option<String>,
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
    /// Re-index current project (auto-detects project root)
    Reindex {
        /// Project root path (defaults to git root or cwd)
        #[arg(short, long)]
        path: Option<String>,
    },
    /// Open interactive graph visualization in browser
    View {
        /// Port to serve on
        #[arg(short = 'P', long, default_value = "7175")]
        port: u16,
        /// Don't open browser automatically
        #[arg(long)]
        no_open: bool,
    },
    /// Touch a file node — increment access_count (used by hooks)
    Touch {
        /// Path to the file
        path: String,
    },
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
        Commands::Note {
            text,
            r#type,
            label,
            project,
        } => commands::note(&text, &r#type, label, project).await,
        Commands::Context { topic, depth } => commands::context(&topic, depth).await,
        Commands::Search { query } => commands::search(&query).await,
        Commands::Sync => commands::sync().await,
        Commands::Reindex { path } => commands::reindex(path).await,
        Commands::View { port, no_open } => view::serve(port, no_open).await,
        Commands::Touch { path } => commands::touch(&path).await,
        Commands::Export => commands::export().await,
        Commands::Mcp => commands::mcp().await,
    }
}
