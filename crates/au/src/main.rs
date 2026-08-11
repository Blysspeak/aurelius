mod commands;
mod view;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::ffi::OsString;

#[derive(Parser)]
#[command(name = "au", about = "Aurelius — personal knowledge graph", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
pub enum TaskAction {
    /// Create a new task
    New {
        /// Task title
        title: String,
        /// Project name
        #[arg(short, long)]
        project: Option<String>,
        /// Priority: critical, high, medium, low
        #[arg(long, default_value = "medium")]
        priority: String,
        /// Acceptance criteria (can be specified multiple times)
        #[arg(short = 'c', long = "criteria")]
        criteria: Vec<String>,
        /// Description
        #[arg(short, long)]
        description: Option<String>,
    },
    /// List tasks
    List {
        /// Filter by project
        #[arg(short, long)]
        project: Option<String>,
        /// Filter by status (comma-separated)
        #[arg(short, long)]
        status: Option<String>,
        /// Filter by priority
        #[arg(long)]
        priority: Option<String>,
    },
    /// Show full task details with work log branch
    Show {
        /// Task UUID or label
        id: String,
    },
    /// Log work done on a task
    Log {
        /// Task UUID or label
        id: String,
        /// Description of work done
        text: String,
    },
    /// Mark task as done
    Done {
        /// Task UUID or label
        id: String,
    },
    /// Block a task with a reason
    Block {
        /// Task UUID or label
        id: String,
        /// Reason for blocking
        reason: String,
    },
    /// Activate a task (set status to active)
    Activate {
        /// Task UUID or label
        id: String,
    },
    /// Show task analytics (completion rate, avg duration, blocked, etc.)
    Stats {
        /// Filter by project
        #[arg(short, long)]
        project: Option<String>,
        /// Window in days for "done_in_window" metric
        #[arg(long)]
        since_days: Option<u64>,
    },
}

#[derive(Subcommand)]
pub enum HomeAction {
    /// Switch the active data/config directory — every au/aurelius
    /// invocation, from any shell, uses it from now on. No AURELIUS_HOME
    /// to re-export by hand every session; useful for running more than
    /// one profile on the same machine (e.g. simulating a collaborator).
    Use {
        /// Directory to use (created if it doesn't exist)
        path: String,
    },
    /// Show which home is currently active
    Current,
    /// Revert to the default OS data/config directories
    Reset,
}

#[derive(Subcommand)]
pub enum IdentityAction {
    /// Set your name and email — stamped as "Name <email>" on every node/edge
    /// you create or update. Required once per machine before `au share
    /// <server> <token>` can be used.
    Set {
        #[arg(long)]
        name: String,
        #[arg(long)]
        email: String,
    },
}

#[derive(Subcommand)]
pub enum DbAction {
    /// Verify database integrity (read-only — never migrates, never writes)
    Check {
        /// Database to check (default: the knowledge graph). Use it to verify a snapshot.
        path: Option<String>,
        /// Report every problem instead of stopping at the first
        #[arg(long)]
        full: bool,
    },
    /// Safe snapshot via SQLite VACUUM INTO — the only correct way to copy a live database
    Backup {
        /// Destination file (default: aurelius-<UTC timestamp>.db next to the database)
        #[arg(short, long)]
        out: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum DocAction {
    /// Convert a document (or a directory of them) to Markdown
    Convert {
        /// File or directory to convert
        path: String,
        /// Write the Markdown here instead of to stdout
        #[arg(short, long)]
        out: Option<String>,
        /// Directory mode: descend into subdirectories
        #[arg(short, long)]
        recursive: bool,
        /// Re-convert even if this content was converted before
        #[arg(long)]
        force: bool,
    },
    /// Search everything ever converted
    Recall {
        /// FTS5 query over document text and file names
        query: String,
        /// Max results
        #[arg(short, long, default_value = "10")]
        limit: usize,
    },
}

#[derive(Subcommand)]
#[command(
    after_help = "The most common command has no subcommand name — it's just:\n\n    au share <SERVER> <TOKEN>\n\nRun that once per project to connect it (using a token from `au share issue`,\nor one someone else gave you). This attaches the project; sync learns the\nname from the server, so there's nothing else to type.\n\nRunning your own server? `au share admin-set <SERVER> <TOKEN>` once, using\nthe AURELIUS_SYNC_ADMIN_TOKEN you gave the server — after that, `issue` and\n`revoke` pick it up automatically, no per-session env var needed."
)]
pub enum ShareAction {
    /// [ADMIN] Store this machine's admin token for a server, so `issue`/
    /// `revoke` don't need AURELIUS_SYNC_ADMIN_TOKEN set every session
    AdminSet {
        /// Sync server host or URL
        server: String,
        /// The admin token the server was started with (AURELIUS_SYNC_ADMIN_TOKEN)
        token: String,
    },
    /// [ADMIN] Issue a collaborator a token for an existing local project
    Issue {
        /// Project label — must already exist locally
        project: String,
        /// Who the token is for, e.g. "Tester <tester@example.com>". Defaults
        /// to your own `au identity set` when issuing a token for yourself
        /// (e.g. to connect this same project from another machine).
        #[arg(long = "for")]
        for_: Option<String>,
        /// Sync server host or URL, e.g. aurelius.boostix.space or http://localhost:8181/sync
        #[arg(long)]
        server: String,
    },
    /// [ADMIN] Revoke a collaborator's access to a project
    Revoke {
        /// Project label
        project: String,
        /// Collaborator email
        #[arg(long = "for")]
        for_: String,
        /// Sync server host or URL
        #[arg(long)]
        server: String,
    },
    /// Push local changes to the sync server
    Push {
        /// Project to push (default: every project with sync enabled)
        project: Option<String>,
    },
    /// Pull changes from the sync server
    Pull {
        /// Project to pull (default: every project with sync enabled)
        project: Option<String>,
    },
    /// List every project connected to sync
    List,
    /// Disable sync for a project (local only — does not delete synced data)
    Disable {
        /// Project label
        project: String,
    },
    /// Connect a project to a sync server, once per project: `au share <server> <token>`
    #[command(external_subcommand)]
    Connect(Vec<OsString>),
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
        /// Show sync conflict details (data._sync_conflict) when present
        #[arg(short, long)]
        verbose: bool,
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
    /// Task management — create, track, and log work on tasks
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Merge two duplicate nodes — rewires edges from source to target, deletes source
    Merge {
        /// Source node (UUID or label) — will be deleted
        source: String,
        /// Target node (UUID or label) — survives with merged edges
        target: String,
    },
    /// Print the skill index (name + trigger) — used by the SessionStart hook
    Skills {
        /// Emit Claude Code SessionStart hook JSON instead of plain text
        #[arg(long)]
        hook: bool,
    },
    /// Seven-layer frozen memory snapshot (Markdown, ~1.5K tokens)
    Snapshot {
        /// Project scope; with --hook defaults to the current folder name
        #[arg(short, long)]
        project: Option<String>,
        /// Emit Claude Code SessionStart hook JSON (never fails, silent on error)
        #[arg(long)]
        hook: bool,
    },
    /// Switch or inspect which data/config directory au/aurelius use
    Home {
        #[command(subcommand)]
        action: HomeAction,
    },
    /// Configure your personal identity for sync attribution
    Identity {
        #[command(subcommand)]
        action: IdentityAction,
    },
    /// Share a project with another Aurelius instance over a sync server
    Share {
        #[command(subcommand)]
        action: ShareAction,
    },
    /// Database maintenance — integrity check and safe backup
    Db {
        #[command(subcommand)]
        action: DbAction,
    },
    /// Convert documents to Markdown, and search what was converted
    Doc {
        #[command(subcommand)]
        action: DocAction,
    },
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
        Commands::Context {
            topic,
            depth,
            verbose,
        } => commands::context(&topic, depth, verbose).await,
        Commands::Search { query } => commands::search(&query).await,
        Commands::Sync => commands::sync().await,
        Commands::Reindex { path } => commands::reindex(path).await,
        Commands::View { port, no_open } => view::serve(port, no_open).await,
        Commands::Touch { path } => commands::touch(&path).await,
        Commands::Export => commands::export().await,
        Commands::Task { action } => commands::task(action).await,
        Commands::Merge { source, target } => commands::merge(&source, &target).await,
        Commands::Skills { hook } => commands::skills(hook).await,
        Commands::Snapshot { project, hook } => commands::snapshot(project, hook).await,
        Commands::Home { action } => commands::home(action).await,
        Commands::Identity { action } => commands::identity(action).await,
        Commands::Share { action } => commands::share(action).await,
        Commands::Db { action } => commands::db(action).await,
        Commands::Doc { action } => commands::doc(action).await,
        Commands::Mcp => commands::mcp().await,
    }
}
