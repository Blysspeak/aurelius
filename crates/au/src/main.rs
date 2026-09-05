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
    /// Edit an existing task: priority, title, description, and added
    /// acceptance criteria. Flags are independent — any combination in one
    /// call; a call with no mutating flag is an error, not a silent no-op.
    /// Criteria are appended, never replaced, and an appended one is stored
    /// exactly like one given to `au task new`
    Update {
        /// Task UUID or label
        id: String,
        /// New priority: critical, high, medium, low
        #[arg(long)]
        priority: Option<String>,
        /// New title — the `[project]` prefix of the label is re-derived, so
        /// project attribution is preserved
        #[arg(long)]
        title: Option<String>,
        /// New description, replacing the current one
        #[arg(short, long)]
        description: Option<String>,
        /// Acceptance criterion to append (can be specified multiple times)
        #[arg(short = 'c', long = "criteria")]
        criteria: Vec<String>,
        /// Print one line of JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// Show full task details with work log branch: prints the three
    /// timestamps (created/started/closed), the resolution and the evidence
    /// (spec 007, FR-002). The contract `contracts/cli.md` calls it
    /// `au task view` — an alias, not a rename (principle VI)
    #[command(alias = "view")]
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
    /// Mark task as done. The resolution is assembled from the traces of the
    /// work — the commit from the repository state, the files from linked
    /// edits; the flags here only refine it (spec 007, FR-004…006). With no
    /// details and without `--unconfirmed` the close still goes through, but
    /// is marked as closed without confirmation (FR-005)
    Done {
        /// Task UUID or label
        id: String,
        /// Commit that resolved the task. When absent, the system tries to
        /// determine it on its own (`git rev-parse --short HEAD`)
        #[arg(long)]
        commit: Option<String>,
        /// Link to the pull request
        #[arg(long = "pr")]
        pull_request: Option<String>,
        /// Explicitly mark "closed without confirmation", even if part of the
        /// resolution was determined on its own
        #[arg(long)]
        unconfirmed: bool,
    },
    /// Block a task with a reason
    Block {
        /// Task UUID or label
        id: String,
        /// Reason for blocking
        reason: String,
    },
    /// Activate a task (set status to active). Evicts the project's
    /// previously active task back into `backlog` — a project holds no more
    /// than one active task (spec 007, FR-031)
    Activate {
        /// Task UUID or label
        id: String,
    },
    /// Attach evidence of a run to a task. Called by the ulika hook
    /// (`record-verify.mjs`), not by a person. The hook knows which project
    /// the run happened in, but not the id of the active task — so instead
    /// of `id` you may name `--project`: the evidence goes to the active
    /// task of that project (spec 007, FR-007…010; FR-008, attaching without
    /// a separate human action; FR-009, never crosses a project boundary —
    /// resolution is strictly by `data.project`)
    Evidence {
        /// Task UUID or label. May be omitted when `--project` is given
        id: Option<String>,
        /// Project whose active task receives the evidence — an alternative
        /// to `id`
        #[arg(long)]
        project: Option<String>,
        /// The command that was run
        #[arg(long)]
        command: String,
        /// Exit code of the run
        #[arg(long)]
        exit: i64,
        /// Path to the artifact of the run, if there is one
        #[arg(long)]
        artifact: Option<String>,
        /// Print a single JSON line instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// Show tasks ripe for closing, each with its grounds: which piece of
    /// evidence, when, and what was changed (spec 007, FR-011…013)
    Ripe {
        /// Filter by project
        #[arg(short, long)]
        project: Option<String>,
        #[arg(long)]
        json: bool,
        /// Decline the offer to close this task — do not present it again
        /// until a new edit appears on it (FR-015)
        #[arg(long)]
        decline: Option<String>,
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
    /// Mark one acceptance criterion of a task met, or unmark it. With
    /// neither flag, lists the task's criteria and the handle each one is
    /// addressed by.
    ///
    /// A criterion is addressed by a stable handle derived from its text, not
    /// by its position in the list: adding a criterion renumbers every one
    /// after it, so a position written down in a script or in an earlier
    /// session points at a different sentence than it did. `#N` is still
    /// accepted for an explicit one-off position.
    ///
    /// Marking a criterion met is a record of progress, not a closing
    /// condition: readiness stays `spec 007, FR-011` — an edit plus a green
    /// run newer than that edit — and closing stays a human decision
    /// (FR-014). Neither `au task ripe` nor `au task done` reads these marks.
    Criterion {
        /// Task UUID or label
        id: String,
        /// Criterion to mark met: a handle prefix, the exact criterion text, or `#N`
        #[arg(long, value_name = "CRITERION")]
        met: Option<String>,
        /// Criterion to unmark, addressed the same way as `--met`
        #[arg(long, value_name = "CRITERION")]
        unmet: Option<String>,
    },
    /// Work order: take one machine work order out of the pool. Only the
    /// shift calls this, not the executor — contract
    /// contracts/au-task-cli.md (spec 006, phase 2)
    Claim {
        /// Who is taking it: `smena@<host>/<pid>`
        #[arg(long)]
        owner: String,
        /// Number of the run that issued the work order
        #[arg(long)]
        run: String,
        /// Lease duration in minutes (twice the wall-clock budget of one
        /// work order)
        #[arg(long = "lease-minutes")]
        lease_minutes: i64,
        /// Filter by project (not part of the wave 1 contract — groundwork
        /// for fitness)
        #[arg(long)]
        project: Option<String>,
        /// JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// Work order: extend the lease on a taken work order. Called by the
    /// shift while the child process is alive — not by the executor (FR-009)
    Renew {
        /// Identifier of the work order issued by `claim`
        #[arg(long)]
        id: String,
        /// The same owner that took the work order
        #[arg(long)]
        owner: String,
        #[arg(long = "lease-minutes")]
        lease_minutes: i64,
        #[arg(long)]
        json: bool,
    },
    /// Work order: report the outcome of the work. The shift makes the
    /// decision — this command only records the report (FR-012)
    Release {
        #[arg(long)]
        id: String,
        #[arg(long)]
        owner: String,
        /// done | failed
        #[arg(long)]
        verdict: String,
        /// Command or check that confirmed the outcome
        #[arg(long)]
        evidence: String,
        #[arg(long)]
        json: bool,
    },
    /// Work order: hand the work order back — the executor recognised that
    /// it is stuck on a human. Blocks the task and does NOT return it to the
    /// queue (FR-014)
    GiveUp {
        #[arg(long)]
        id: String,
        #[arg(long)]
        owner: String,
        /// Reason why the task cannot be closed by a machine
        #[arg(long)]
        why: String,
        #[arg(long)]
        json: bool,
    },
    /// Work order: set a verdict on executability, or dry-run the labelling
    /// across the whole queue. Writes only `data.fitness` — contract
    /// `au-task-cli.md`, section `au task fitness` (FR-003, spec 006,
    /// phase 3)
    Fitness {
        /// Task identifier — together with --verdict and --why it sets the
        /// verdict by hand. Required unless --dry-run is given
        #[arg(long)]
        id: Option<String>,
        /// machine | human | split — required together with --id
        #[arg(long)]
        verdict: Option<String>,
        /// Rationale — required and non-empty together with --id (FR-003a)
        #[arg(long)]
        why: Option<String>,
        /// Dry run across all open tasks — writes nothing (SC-001)
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Filter by project — takes effect only together with --dry-run
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

/// Координаты секретов (спека 007, US4). Хранится место, где лежит секрет —
/// не сам секрет: FR-025 запрещает хранить значение в любом виде, включая
/// зашифрованный.
#[derive(Subcommand)]
pub enum SecretAction {
    /// Записать координату секрета. Строка `--where` проверяется на признаки
    /// «похоже на само значение» (FR-026) — совпадение отклоняет запись с
    /// кодом 1 и объяснением, какой признак сработал.
    Add {
        /// Имя секрета, например STRIPE_SECRET_KEY
        #[arg(long)]
        name: String,
        /// Место хранения: переменная окружения, путь к файлу или запись в
        /// менеджере паролей — НЕ само значение
        #[arg(long = "where")]
        location: String,
        /// Назначение секрета человеческим языком
        #[arg(long)]
        purpose: Option<String>,
        /// Проект, которому принадлежит секрет
        #[arg(long)]
        project: Option<String>,
    },
    /// Показать записанные координаты — без значений, их здесь и не было
    List {
        /// Filter by project
        #[arg(short, long)]
        project: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Удалить координату по имени
    Rm {
        /// Имя секрета
        name: String,
        #[arg(long)]
        project: Option<String>,
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

/// Bulk graph operations for external vendor documentation (spec 008).
#[derive(Subcommand)]
pub enum GraphAction {
    /// Import a `graph.json` file (nodes + edges) in one transaction — either
    /// everything lands, or nothing does (FR-001..FR-006). Re-running the
    /// same file is a no-op; a changed body updates only that node (FR-002).
    Import {
        /// Path to the graph.json file (see specs/008-doc-graph/spec.md,
        /// "Key Entities", for the shape)
        file: String,
        /// Print the report as one JSON line instead of human-readable text
        #[arg(long)]
        json: bool,
    },
    /// Print a (sub)graph in mermaid syntax (FR-015). `--format json` prints
    /// the same full dump as the bare `au export` command.
    Export {
        /// json | mermaid
        #[arg(long, default_value = "json")]
        format: String,
        /// Mermaid only: restrict to one import's nodes (`data.source_id`)
        /// and the edges between them, instead of the whole graph
        #[arg(long = "source-id")]
        source_id: Option<String>,
    },
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize Aurelius in current environment
    Init,
    /// Add a knowledge node manually
    Note(commands::NoteArgs),
    /// Record what happened this session — the same record `memory_session`
    /// writes over MCP, and the only one layer 4 of the snapshot
    /// ("Последние сессии") reads. A note lands in layer 5, among lasting facts.
    Session(commands::SessionArgs),
    /// Link two nodes with a typed edge — what `memory_relate` does over MCP.
    /// Without it everything a hook writes lands in the graph edgeless.
    Relate(commands::RelateArgs),
    /// List what a given run wrote. A session-end hook needs this to tell its
    /// own records from yesterday's — until now nothing carried the run.
    Journal(commands::JournalArgs),
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
    /// Read one record back by exact key — a node UUID, or the exact
    /// `--subject` a fact was written with. Nothing here is fuzzy: `search`
    /// and `context` go through the full-text index, which does not cover the
    /// `id` column at all, and would answer a mistyped key with a neighbour
    /// instead of a miss.
    Recall(commands::RecallArgs),
    /// Изъята (спека 007, US5, T047, `contracts/cli.md` §«Изымается»):
    /// TimeForged-коннектор не имел ни одного вызова ни в хуках, ни в
    /// журналах 19 репозиториев (разведка 30.08.2026). Подкоманда остаётся
    /// разбираемой — старый вызов получает внятное сообщение и код 1, а не
    /// ошибку разбора аргументов clap.
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
    /// Координаты секретов проекта — место хранения, не значение (спека 007, US4)
    Secret {
        #[command(subcommand)]
        action: SecretAction,
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
    /// Append an action trace (Bit-i-Delo stage 1); --hook reads Claude hook JSON from stdin
    Trace {
        /// tool_call|file_edit|error|commit|msg_sent|user_correction
        #[arg(short, long)]
        kind: Option<String>,
        /// Trace payload text
        #[arg(short = 'm', long)]
        payload: Option<String>,
        /// Session id (defaults to "cli")
        #[arg(short, long)]
        session: Option<String>,
        /// Exit code of the traced action
        #[arg(short, long)]
        exit_code: Option<i64>,
        /// PostToolUse hook mode: parse hook JSON from stdin, never fail
        #[arg(long)]
        hook: bool,
    },
    /// Изъята (спека 007, US5, T046, `contracts/cli.md` §«Изымается»): хук,
    /// который её звал, не подключён ни в одном проекте (разведка
    /// 30.08.2026). Флаги ниже приняты только ради совместимости разбора —
    /// подкоманда остаётся разбираемой, старый вызов получает внятное
    /// сообщение об изъятии и код 1, а не ошибку разбора аргументов clap.
    Capture {
        #[arg(long)]
        hook: bool,
        #[arg(short, long, conflicts_with = "hook")]
        command: Option<String>,
    },
    /// Close ripe labile windows and apply outcome verdicts (Bit-i-Delo stage 4)
    Judge {
        /// Only close windows at least this many seconds old (default 0)
        #[arg(short, long, default_value = "0")]
        min_age: i64,
        /// Stop-hook mode: never fail
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
        /// Emit machine-readable facts instead of Markdown:
        /// {"project":…,"facts":[{"kind","text","at"}]}. An empty `facts` with
        /// exit 0 means "nothing to say"; no output or a non-zero exit means
        /// "broken". Parsing the Markdown means parsing the layout.
        #[arg(long, conflicts_with = "hook")]
        json: bool,
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
    /// Bulk import and mermaid export of an external documentation graph
    /// (spec 008) — hundreds of nodes/edges in one call instead of one
    /// `au note`/`au relate` per page.
    Graph {
        #[command(subcommand)]
        action: GraphAction,
    },
    /// Directed step ladder over `next_step`/`prerequisite` edges (spec 008,
    /// FR-008..FR-010): shortest path between two nodes, or every ancestor of
    /// one node in topological order. Exactly one of the two forms — pass
    /// both `from` and `to`, or `--before`, never neither nor both.
    Path {
        /// Start node: UUID, exact `subject`, or exact `label`. Required
        /// unless `--before` is given
        from: Option<String>,
        /// End node — same forms as `from`. Required unless `--before` is given
        to: Option<String>,
        /// List every node that transitively leads to X, in "earliest first"
        /// order, instead of a path between two named nodes
        #[arg(long, conflicts_with_all = ["from", "to"])]
        before: Option<String>,
        /// Walk depth cap — guards against a runaway search on a malformed graph
        #[arg(long, default_value = "50")]
        max_depth: usize,
        /// Print machine-readable JSON instead of the human-readable ladder
        #[arg(long)]
        json: bool,
    },
}

/// Договор о кодах возврата. Вызывающий обязан различать «я позвал
/// неправильно» и «база недоступна»: первое чинится другим вызовом, второе —
/// руками, и переспрашивать бессмысленно. Раньше оба случая давали единицу, а
/// clap на опечатку в аргументе отдавал двойку — ровно наоборот.
mod exit {
    /// Ошибка вызова: неизвестный тип, кривые аргументы, ненайденный узел.
    pub const USAGE: u8 = 1;
    /// Хранилище недоступно: нет базы, битый образ, залоченный SQLite.
    pub const STORAGE: u8 = 2;
    /// Наряд (`au task claim|renew|release|give-up`, контракт
    /// `au-task-cli.md`): пул пуст, либо наряд больше не принадлежит
    /// вызывающему (аренда истекла и перевыдана). Не ошибка хранилища и не
    /// ошибка вызова — смена считает подряд идущие ответы с этим кодом,
    /// чтобы понять, что очередь исчерпана.
    pub const LEASE_EMPTY: u8 = 10;
    /// Наряд: база занята другим писателем. FR-010 требует отличать это от
    /// `LEASE_EMPTY` — иначе ручная сессия владельца выглядит как пустая
    /// очередь, и смена выходит, отчитавшись об успехе.
    pub const LEASE_BUSY: u8 = 11;
}

/// Хранилищем считается всё, что пришло из слоя базы: `DbError` (открытие,
/// миграция, целостность) и голая ошибка `rusqlite` из любого запроса. Всё
/// остальное — ошибка вызова. Наряд проверяется первым: `LeaseError` может
/// нести `rusqlite::Error` источником (`Busy`), и без этой проверки он
/// попал бы в `STORAGE`, а контракт `au-task-cli.md` требует для наряда
/// ровно четыре кода — 0, 10, 11, 1.
fn classify(err: &anyhow::Error) -> u8 {
    if let Some(lease_err) = err
        .chain()
        .find_map(|c| c.downcast_ref::<aurelius_core::graph::LeaseError>())
    {
        return match lease_err {
            aurelius_core::graph::LeaseError::Busy(_) => exit::LEASE_BUSY,
            aurelius_core::graph::LeaseError::NoTasksAvailable
            | aurelius_core::graph::LeaseError::NotOwner => exit::LEASE_EMPTY,
        };
    }
    let storage = err
        .chain()
        .any(|c| c.is::<aurelius_core::db::DbError>() || c.is::<rusqlite::Error>());
    if storage {
        exit::STORAGE
    } else {
        exit::USAGE
    }
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            // `--help` и `--version` — это не сбой вызова, это запрошенный вывод.
            return match e.kind() {
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
                    std::process::ExitCode::SUCCESS
                }
                _ => std::process::ExitCode::from(exit::USAGE),
            };
        }
    };
    match run(cli).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Ошибка: {e:#}");
            std::process::ExitCode::from(classify(&e))
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Init => commands::init().await,
        Commands::Note(args) => commands::note(args).await,
        Commands::Session(args) => commands::session(args).await,
        Commands::Relate(args) => commands::relate(args).await,
        Commands::Journal(args) => commands::journal(args).await,
        Commands::Context {
            topic,
            depth,
            verbose,
        } => commands::context(&topic, depth, verbose).await,
        Commands::Search { query } => commands::search(&query).await,
        Commands::Recall(args) => commands::recall(args).await,
        Commands::Sync => {
            commands::removed(
                "sync",
                "TimeForged-коннектор не имел ни одного вызова ни в хуках, ни в журналах 19 репозиториев",
            )
            .await
        }
        Commands::Reindex { path } => commands::reindex(path).await,
        Commands::View { port, no_open } => view::serve(port, no_open).await,
        Commands::Touch { path } => commands::touch(&path).await,
        Commands::Export => commands::export().await,
        Commands::Task { action } => commands::task(action).await,
        Commands::Secret { action } => commands::secret(action).await,
        Commands::Merge { source, target } => commands::merge(&source, &target).await,
        Commands::Skills { hook } => commands::skills(hook).await,
        Commands::Trace {
            kind,
            payload,
            session,
            exit_code,
            hook,
        } => commands::trace_cmd(kind, payload, session, exit_code, hook).await,
        Commands::Capture { hook, command } => {
            // Флаги старого вызова принимаются разбором (иначе это была бы
            // ошибка разбора аргументов, которую запрещает T048), но здесь
            // только называются в сообщении — команда ничего не делает.
            let reason = format!(
                "хук, который её звал, не подключён ни в одном проекте (--hook={hook}, --command={command:?})"
            );
            commands::removed("capture", &reason).await
        }
        Commands::Judge { min_age, hook } => commands::judge_cmd(min_age, hook).await,
        Commands::Snapshot {
            project,
            hook,
            json,
        } => commands::snapshot(project, hook, json).await,
        Commands::Home { action } => commands::home(action).await,
        Commands::Identity { action } => commands::identity(action).await,
        Commands::Share { action } => commands::share(action).await,
        Commands::Db { action } => commands::db(action).await,
        Commands::Doc { action } => commands::doc(action).await,
        Commands::Mcp => commands::mcp().await,
        Commands::Graph { action } => commands::graph(action).await,
        Commands::Path {
            from,
            to,
            before,
            max_depth,
            json,
        } => commands::path(from, to, before, max_depth, json).await,
    }
}
