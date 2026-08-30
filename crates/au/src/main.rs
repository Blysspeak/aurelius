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
    /// Show full task details with work log branch. Спека 007, FR-002:
    /// печатает три времени (заведена/взята/закрыта), способ решения и
    /// улики. Контракт `contracts/cli.md` называет её `au task view` —
    /// алиас, а не переименование (принцип VI)
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
    /// Mark task as done. Способ решения (спека 007, FR-004…006) собирается
    /// из следов работы — коммит из состояния репозитория, файлы из
    /// привязанных правок; флаги здесь только уточняют. Без сведений и без
    /// `--unconfirmed` закрытие всё равно проходит, но помечается как
    /// закрытое без подтверждения (FR-005)
    Done {
        /// Task UUID or label
        id: String,
        /// Коммит, которым решена задача. При отсутствии система пытается
        /// определить его сама (`git rev-parse --short HEAD`)
        #[arg(long)]
        commit: Option<String>,
        /// Ссылка на запрос на слияние
        #[arg(long = "pr")]
        pull_request: Option<String>,
        /// Явно пометить «закрыта без подтверждения», даже если что-то из
        /// способа решения удалось определить самостоятельно
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
    /// Activate a task (set status to active). Вытесняет прежнюю активную
    /// задачу проекта в `backlog` — в проекте не более одной активной
    /// (спека 007, FR-031)
    Activate {
        /// Task UUID or label
        id: String,
    },
    /// Привязать улику прогона к задаче (спека 007, FR-007…010). Зовётся
    /// хуком ulika (`record-verify.mjs`), не человеком. Хук знает, в каком
    /// проекте состоялся прогон, но не id активной задачи — поэтому вместо
    /// `id` можно назвать `--project`: улика уйдёт активной задаче этого
    /// проекта (FR-008, привязка без отдельного действия человека; FR-009,
    /// не пересекает границу проекта — resolve строго по `data.project`)
    Evidence {
        /// Task UUID or label. Можно опустить, если задан `--project`
        id: Option<String>,
        /// Проект, чья активная задача получит улику — альтернатива `id`
        #[arg(long)]
        project: Option<String>,
        /// Прогнанная команда
        #[arg(long)]
        command: String,
        /// Код возврата прогона
        #[arg(long)]
        exit: i64,
        /// Путь к артефакту прогона, если он есть
        #[arg(long)]
        artifact: Option<String>,
        /// Печатать одну строку JSON вместо человекочитаемого текста
        #[arg(long)]
        json: bool,
    },
    /// Показать созревшие к закрытию задачи с основанием: какая улика, когда,
    /// что изменено (спека 007, FR-011…013)
    Ripe {
        /// Filter by project
        #[arg(short, long)]
        project: Option<String>,
        #[arg(long)]
        json: bool,
        /// Отклонить предложение закрыть эту задачу — не предъявлять снова,
        /// пока по ней не появится новая правка (FR-015)
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
    /// Наряд: взять один машинный наряд из пула (спека 006, фаза 2). Только
    /// смена вызывает это, не исполнитель — контракт contracts/au-task-cli.md
    Claim {
        /// Кто берёт: `smena@<host>/<pid>`
        #[arg(long)]
        owner: String,
        /// Номер прогона, выдавшего наряд
        #[arg(long)]
        run: String,
        /// Срок аренды в минутах (вдвое больше стены по времени на наряд)
        #[arg(long = "lease-minutes")]
        lease_minutes: i64,
        /// Фильтр по проекту (не входит в контракт волны 1 — задел на fitness)
        #[arg(long)]
        project: Option<String>,
        /// JSON вместо человекочитаемого текста
        #[arg(long)]
        json: bool,
    },
    /// Наряд: продлить аренду взятого наряда. Вызывает смена, пока дочерний
    /// процесс жив, — не исполнитель (FR-009)
    Renew {
        /// Идентификатор наряда, выданный `claim`
        #[arg(long)]
        id: String,
        /// Тот же владелец, что взял наряд
        #[arg(long)]
        owner: String,
        #[arg(long = "lease-minutes")]
        lease_minutes: i64,
        #[arg(long)]
        json: bool,
    },
    /// Наряд: заявить исход работы. Решение принимает смена — эта команда
    /// только записывает заявку (FR-012)
    Release {
        #[arg(long)]
        id: String,
        #[arg(long)]
        owner: String,
        /// done | failed
        #[arg(long)]
        verdict: String,
        /// Команда или проверка, которой подтверждён исход
        #[arg(long)]
        evidence: String,
        #[arg(long)]
        json: bool,
    },
    /// Наряд: сдать наряд — исполнитель распознал упор в человека. Блокирует
    /// задачу и НЕ возвращает её в очередь (FR-014)
    GiveUp {
        #[arg(long)]
        id: String,
        #[arg(long)]
        owner: String,
        /// Причина, по которой задача не может быть закрыта машиной
        #[arg(long)]
        why: String,
        #[arg(long)]
        json: bool,
    },
    /// Наряд: поставить вердикт исполнимости, либо сухо прогнать разметку по
    /// всей очереди. Пишет только `data.fitness` — контракт
    /// `au-task-cli.md`, раздел `au task fitness` (FR-003, спека 006, фаза 3)
    Fitness {
        /// Идентификатор задачи — вместе с --verdict и --why ставит вердикт
        /// вручную. Обязателен без --dry-run
        #[arg(long)]
        id: Option<String>,
        /// machine | human | split — обязателен вместе с --id
        #[arg(long)]
        verdict: Option<String>,
        /// Обоснование — обязательно и непусто вместе с --id (FR-003a)
        #[arg(long)]
        why: Option<String>,
        /// Сухой прогон по всем открытым задачам — ничего не пишет (SC-001)
        #[arg(long = "dry-run")]
        dry_run: bool,
        /// Фильтр по проекту — действует только вместе с --dry-run
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
    }
}
