//! Типизированные поля задачи (спека 007, фаза 2): времена, способ решения и
//! улики, которые сегодня лежат вперемешку с прочим в `Node.data` узла типа
//! `Task` (`status`, `priority`, поля аренды из `graph::lease`).
//!
//! Схема БД не меняется (принцип I) — все поля ниже лишь новые ключи в том же
//! `data`. Узел, заведённый до этой фичи, не содержит ни одного из них:
//! [`TaskFields::from_data`] обязана прочитать такой узел без ошибки и отдать
//! пустые поля (T005). Обратная запись — [`TaskFields::merge_into`] — обязана
//! не терять посторонние ключи чужих модулей (T006): она стартует с исходной
//! карты `data` и только перезаписывает свои шесть ключей.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Способ решения задачи (`data.resolution`), пишется при закрытии.
/// `confirmed: false` — «закрыта без подтверждения»: способ решения неизвестен,
/// и это записано явно, а не подразумевается пустотой (data-model.md).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Resolution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default)]
    pub confirmed: bool,
}

/// Одна улика прогона (элемент `data.evidence`).
///
/// `artifact_present` — `Option`, а не `bool`: улики, привязанные до проверки
/// наличия файла (или до этой фичи вовсе), не обязаны иметь мнение на этот
/// счёт. `None` значит «не проверялось», а не «файл есть».
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceEntry {
    pub command: String,
    pub exit_code: i64,
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_present: Option<bool>,
}

/// Все новые поля задачи из `data-model.md`, все опциональны. Задача без них
/// — обычная задача, заведённая до этой фичи.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Resolution>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_edit_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declined_ripe_at: Option<DateTime<Utc>>,
}

impl TaskFields {
    /// Читает шесть полей из `Node.data`, игнорируя все остальные ключи,
    /// которые там лежат (`status`, `priority`, `lease`, `attempts`, ...).
    /// Узел без единого нового ключа даёт `TaskFields::default()` — не
    /// ошибку (T005).
    pub fn from_data(data: &Value) -> Self {
        serde_json::from_value(data.clone()).unwrap_or_default()
    }

    /// Сливает свои шесть полей обратно в `data`, не трогая ничего постороннее
    /// (T006). Работает всегда от исходной карты: значение, отсутствующее в
    /// `self` (сериализация пропускает `None`/пустой `Vec` через
    /// `skip_serializing_if`), просто не упоминается в патче и остаётся в
    /// `data` тем, чем было.
    ///
    /// `data`, не являющийся объектом (пустой узел, миграция чего-то иного),
    /// заменяется новым объектом целиком — терять посторонние ключи там
    /// физически не из чего.
    pub fn merge_into(&self, data: &Value) -> Value {
        let mut map = match data {
            Value::Object(m) => m.clone(),
            _ => serde_json::Map::new(),
        };
        let patch = match serde_json::to_value(self) {
            Ok(Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        };
        for (key, value) in patch {
            map.insert(key, value);
        }
        Value::Object(map)
    }
}

/// Коммит, которым решается задача, если способ решения не назвали явно
/// (T021a, FR-006): `git rev-parse --short HEAD` в текущем каталоге. `None`,
/// если это не git-репозиторий или команда недоступна — не повод отказать в
/// закрытии, только не сможем назвать коммит.
///
/// Общая точка для CLI (`au task done`) и MCP (`task_update`, статус `done`)
/// — то же самое правило, вызванное из обоих мест, а не продублированное.
pub fn current_commit_sha() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

/// Собирает способ решения из следов работы (T021a, FR-004…006): коммит — из
/// состояния репозитория, если не назван явно; файлы — из правок,
/// привязанных хуком `au trace --hook` с момента взятия задачи в работу.
/// `commit`/`pull_request` уточняют автоматически собранное, а не заменяют
/// его (FR-006: не спрашивать у человека то, что система уже знает).
/// `unconfirmed` форсирует пометку «без подтверждения»; без него она ставится
/// сама, когда следов не нашлось ни одного (FR-005).
///
/// Общая точка для CLI (`au task done`) и MCP (`task_update`, статус `done`).
pub fn build_resolution(
    conn: &rusqlite::Connection,
    since: DateTime<Utc>,
    commit: Option<String>,
    pull_request: Option<String>,
    unconfirmed: bool,
) -> Resolution {
    let commit = commit.or_else(current_commit_sha);
    let files = crate::trace::files_edited_since(conn, since.timestamp()).unwrap_or_default();
    let confirmed =
        !unconfirmed && (commit.is_some() || pull_request.is_some() || !files.is_empty());
    Resolution {
        commit,
        pull_request,
        files,
        confirmed,
    }
}

/// Возвращает улику, дающую созревание, если она есть — основание для
/// предъявления (какая улика, когда). `None` значит «не созрела».
///
/// Условия созревания (data-model.md, «Производное состояние: созревшая»),
/// все четыре разом:
/// 1. `status == "active"`;
/// 2. есть `last_edit_at`;
/// 3. в `evidence` есть элемент с `exit_code == 0` и `at` позже `last_edit_at`;
/// 4. `declined_ripe_at` отсутствует или старше `last_edit_at`.
///
/// Состояние не хранится — вычисляется каждый раз из уже прочитанных полей.
pub fn ripe_evidence<'a>(fields: &'a TaskFields, status: &str) -> Option<&'a EvidenceEntry> {
    if status != "active" {
        return None;
    }
    let last_edit_at = fields.last_edit_at?;
    if let Some(declined_at) = fields.declined_ripe_at {
        if declined_at >= last_edit_at {
            return None;
        }
    }
    fields
        .evidence
        .iter()
        .filter(|e| e.exit_code == 0 && e.at > last_edit_at)
        .max_by_key(|e| e.at)
}

/// `true`, если задача созрела к закрытию — см. [`ripe_evidence`].
pub fn is_ripe(fields: &TaskFields, status: &str) -> bool {
    ripe_evidence(fields, status).is_some()
}

/// Одна созревшая задача с основанием предъявления (T018, FR-013): какая
/// улика дала созревание, когда, что изменено с момента взятия в работу.
///
/// Общая точка для CLI (`au task ripe`, блок в `au judge --hook`) и MCP
/// (`task_ripe`) — раньше это вычисление сидело приватной функцией внутри
/// `au/src/commands.rs`, и MCP-стороне позвать было нечего: у ассистента нет
/// текстового `--hook`, которым это доходит до человека, а закрывает задачи
/// через MCP тоже ассистент (`task_update`).
#[derive(Debug, Clone)]
pub struct RipeReport {
    pub id: uuid::Uuid,
    pub label: String,
    pub evidence: EvidenceEntry,
    pub files: Vec<String>,
}

/// Собирает созревшие задачи проекта (`None` — по всем проектам): активные
/// задачи, у которых `ripe_evidence` нашла подтверждающую улику, плюс список
/// файлов, изменённых с момента взятия в работу (тот же источник, что и
/// `build_resolution`).
pub fn gather_ripe(
    conn: &rusqlite::Connection,
    project: Option<&str>,
) -> anyhow::Result<Vec<RipeReport>> {
    let active = crate::graph::get_tasks_filtered(conn, project, Some("active"), None, 200)?;
    let mut out = Vec::new();
    for t in active {
        let fields = TaskFields::from_data(&t.data);
        let Some(evidence) = ripe_evidence(&fields, "active") else {
            continue;
        };
        let since = fields.activated_at.unwrap_or(t.created_at);
        let files = crate::trace::files_edited_since(conn, since.timestamp()).unwrap_or_default();
        out.push(RipeReport {
            id: t.id,
            label: t.label.clone(),
            evidence: evidence.clone(),
            files,
        });
    }
    Ok(out)
}

/// Сериализует созревшие задачи в JSON — общая точка для `au task ripe
/// --json` и MCP `task_ripe`, чтобы формы ответа не разъезжались.
pub fn ripe_to_json(ripe: &[RipeReport]) -> Value {
    json!(ripe
        .iter()
        .map(|r| json!({
            "id": r.id.to_string(),
            "label": r.label,
            "evidence": {
                "command": r.evidence.command,
                "exit_code": r.evidence.exit_code,
                "at": r.evidence.at.to_rfc3339(),
                "artifact": r.evidence.artifact,
            },
            "files": r.files,
        }))
        .collect::<Vec<_>>())
}

/// Помечает `artifact_present: false` у улик, чей файл артефакта больше не
/// найден на диске: команда, код возврата и время — не трогаются, честно
/// сохраняется только утрата ссылки (T021d, FR-010). Улики без пути к
/// артефакту не трогает — им нечего проверять.
pub fn refresh_artifact_presence(evidence: &mut [EvidenceEntry]) {
    for entry in evidence.iter_mut() {
        if let Some(path) = &entry.artifact {
            entry.artifact_present = Some(std::path::Path::new(path).exists());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// T005: узел задачи, созданный до фичи (без новых ключей), читается без
    /// ошибки и даёт пустые поля.
    #[test]
    fn from_data_reads_legacy_node_without_error() {
        let data = json!({
            "status": "backlog",
            "priority": "high",
        });

        let fields = TaskFields::from_data(&data);

        assert_eq!(fields, TaskFields::default());
    }

    /// T006: запись новых полей не затирает посторонние ключи в `data` —
    /// ни известный (`priority`), ни произвольный, о котором этот модуль
    /// ничего не знает.
    #[test]
    fn merge_into_preserves_foreign_keys() {
        let data = json!({
            "status": "active",
            "priority": "critical",
            "custom_key_from_another_module": {"nested": true},
        });

        let mut fields = TaskFields::from_data(&data);
        fields.last_edit_at = Some("2026-08-30T09:00:00Z".parse().expect("rfc3339"));

        let merged = fields.merge_into(&data);

        assert_eq!(merged["status"], "active");
        assert_eq!(merged["priority"], "critical");
        assert_eq!(merged["custom_key_from_another_module"]["nested"], true);
        assert_eq!(merged["last_edit_at"], "2026-08-30T09:00:00Z");
    }

    fn evidence(command: &str, exit_code: i64, at: &str) -> EvidenceEntry {
        EvidenceEntry {
            command: command.to_owned(),
            exit_code,
            at: at.parse().expect("rfc3339"),
            artifact: None,
            artifact_present: None,
        }
    }

    /// T022: улика старше последней правки не даёт созревания.
    #[test]
    fn evidence_older_than_last_edit_is_not_ripe() {
        let fields = TaskFields {
            last_edit_at: Some("2026-08-30T10:00:00Z".parse().expect("rfc3339")),
            evidence: vec![evidence("cargo test", 0, "2026-08-30T09:00:00Z")],
            ..Default::default()
        };

        assert!(!is_ripe(&fields, "active"));
    }

    /// T023: улика с ненулевым кодом возврата не даёт созревания.
    #[test]
    fn evidence_with_nonzero_exit_code_is_not_ripe() {
        let fields = TaskFields {
            last_edit_at: Some("2026-08-30T09:00:00Z".parse().expect("rfc3339")),
            evidence: vec![evidence("cargo test", 1, "2026-08-30T10:00:00Z")],
            ..Default::default()
        };

        assert!(!is_ripe(&fields, "active"));
    }

    /// T024: после отказа задача не предъявляется повторно, пока не появится
    /// новая правка — но появление новой правки после отказа возвращает
    /// созревание (условие 4 из data-model.md).
    #[test]
    fn declined_ripe_blocks_until_new_edit() {
        let mut fields = TaskFields {
            last_edit_at: Some("2026-08-30T09:00:00Z".parse().expect("rfc3339")),
            evidence: vec![evidence("cargo test", 0, "2026-08-30T10:00:00Z")],
            declined_ripe_at: Some("2026-08-30T10:05:00Z".parse().expect("rfc3339")),
            ..Default::default()
        };
        assert!(!is_ripe(&fields, "active"));

        // Новая правка после отказа — предложение снова уместно.
        fields.last_edit_at = Some("2026-08-30T11:00:00Z".parse().expect("rfc3339"));
        fields.evidence = vec![evidence("cargo test", 0, "2026-08-30T11:30:00Z")];
        assert!(is_ripe(&fields, "active"));
    }

    #[test]
    fn not_ripe_when_status_is_not_active() {
        let fields = TaskFields {
            last_edit_at: Some("2026-08-30T09:00:00Z".parse().expect("rfc3339")),
            evidence: vec![evidence("cargo test", 0, "2026-08-30T10:00:00Z")],
            ..Default::default()
        };

        assert!(!is_ripe(&fields, "backlog"));
    }

    #[test]
    fn not_ripe_without_last_edit_at() {
        let fields = TaskFields {
            evidence: vec![evidence("cargo test", 0, "2026-08-30T10:00:00Z")],
            ..Default::default()
        };

        assert!(!is_ripe(&fields, "active"));
    }

    /// T021d: файл артефакта отсутствует — помечается `artifact_present:
    /// false`, команда, код возврата и время не трогаются.
    #[test]
    fn refresh_artifact_presence_marks_missing_file() {
        let mut evidence = vec![EvidenceEntry {
            command: "cargo test --workspace".to_owned(),
            exit_code: 0,
            at: "2026-08-30T09:14:22Z".parse().expect("rfc3339"),
            artifact: Some("does/not/exist-on-disk.log".to_owned()),
            artifact_present: Some(true),
        }];

        refresh_artifact_presence(&mut evidence);

        assert_eq!(evidence[0].artifact_present, Some(false));
        assert_eq!(evidence[0].command, "cargo test --workspace");
        assert_eq!(evidence[0].exit_code, 0);
        assert_eq!(
            evidence[0].at,
            "2026-08-30T09:14:22Z"
                .parse::<DateTime<Utc>>()
                .expect("rfc3339")
        );
    }

    #[test]
    fn refresh_artifact_presence_marks_existing_file() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("aurelius-tasks-test-{}.log", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"ok").expect("write temp artifact");

        let mut evidence = vec![EvidenceEntry {
            command: "cargo test".to_owned(),
            exit_code: 0,
            at: Utc::now(),
            artifact: Some(path.to_string_lossy().into_owned()),
            artifact_present: None,
        }];

        refresh_artifact_presence(&mut evidence);

        assert_eq!(evidence[0].artifact_present, Some(true));

        std::fs::remove_file(&path).expect("cleanup temp artifact");
    }

    // -- gather_ripe: перенесено из `au/src/commands.rs`, чтобы CLI (`au task
    // ripe`) и MCP (`task_ripe`) звали одну функцию, а не две копии правила.

    /// Тот же приём, что в `graph::lease` тестах: настоящий temp-файл, не
    /// `:memory:` — `db::open` жёстко требует WAL.
    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn new(tag: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "aurelius-tasks-gather-ripe-{tag}-{}.db",
                uuid::Uuid::new_v4()
            )))
        }
    }

    impl Drop for TmpDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut p = self.0.as_os_str().to_owned();
                p.push(suffix);
                let _ = std::fs::remove_file(std::path::PathBuf::from(p));
            }
        }
    }

    fn setup() -> (TmpDb, rusqlite::Connection) {
        let tmp = TmpDb::new("setup");
        let conn = crate::db::open(&tmp.0).expect("open temp db");
        (tmp, conn)
    }

    fn seed_task(conn: &rusqlite::Connection, label: &str, data: Value) -> uuid::Uuid {
        crate::graph::add_node_full(
            conn,
            crate::models::NodeType::Task,
            label,
            None,
            "test",
            data,
            crate::models::MemoryKind::Semantic,
            None,
        )
        .expect("insert task")
        .id
    }

    /// Активная задача с зелёной уликой новее последней правки — созревшая:
    /// `gather_ripe` обязана вернуть её вместе с уликой, давшей созревание.
    #[test]
    fn gather_ripe_includes_active_task_with_fresh_green_evidence() {
        let (_tmp, conn) = setup();
        let task_id = seed_task(
            &conn,
            "созревшая задача",
            json!({
                "status": "active",
                "priority": "medium",
                "last_edit_at": "2026-08-30T09:00:00Z",
                "evidence": [{
                    "command": "cargo test",
                    "exit_code": 0,
                    "at": "2026-08-30T10:00:00Z",
                }],
            }),
        );

        let ripe = gather_ripe(&conn, None).expect("gather_ripe");

        assert_eq!(ripe.len(), 1);
        assert_eq!(ripe[0].id, task_id);
        assert_eq!(ripe[0].evidence.command, "cargo test");
        assert_eq!(ripe[0].evidence.exit_code, 0);
    }

    /// Асимметрия предыдущего теста: активная задача БЕЗ улики новее правки —
    /// не созревшая, `gather_ripe` обязана её пропустить, а не вернуть с
    /// пустым основанием.
    #[test]
    fn gather_ripe_excludes_active_task_without_fresh_evidence() {
        let (_tmp, conn) = setup();
        seed_task(
            &conn,
            "не созревшая задача",
            json!({
                "status": "active",
                "priority": "medium",
                "last_edit_at": "2026-08-30T09:00:00Z",
            }),
        );

        let ripe = gather_ripe(&conn, None).expect("gather_ripe");

        assert!(
            ripe.is_empty(),
            "задача без свежей улики не обязана предъявляться: {ripe:?}"
        );
    }

    /// Асимметрия по статусу: задача из `backlog` с той же уликой — не
    /// предъявляется, `gather_ripe` фильтрует по `status == "active"` на
    /// уровне запроса, а не только внутри `ripe_evidence`.
    #[test]
    fn gather_ripe_excludes_backlog_task_even_with_evidence() {
        let (_tmp, conn) = setup();
        seed_task(
            &conn,
            "задача в бэклоге",
            json!({
                "status": "backlog",
                "priority": "medium",
                "last_edit_at": "2026-08-30T09:00:00Z",
                "evidence": [{
                    "command": "cargo test",
                    "exit_code": 0,
                    "at": "2026-08-30T10:00:00Z",
                }],
            }),
        );

        let ripe = gather_ripe(&conn, None).expect("gather_ripe");

        assert!(ripe.is_empty());
    }

    /// `ripe_to_json` — форма ответа, общая для CLI и MCP: id строкой, вложенный
    /// `evidence` со всеми четырьмя полями, `files` массивом.
    #[test]
    fn ripe_to_json_serializes_id_as_string_and_nests_evidence() {
        let (_tmp, conn) = setup();
        let task_id = seed_task(
            &conn,
            "задача для сериализации",
            json!({
                "status": "active",
                "priority": "medium",
                "last_edit_at": "2026-08-30T09:00:00Z",
                "evidence": [{
                    "command": "cargo test",
                    "exit_code": 0,
                    "at": "2026-08-30T10:00:00Z",
                    "artifact": "run.log",
                }],
            }),
        );

        let ripe = gather_ripe(&conn, None).expect("gather_ripe");
        let out = ripe_to_json(&ripe);

        assert_eq!(out[0]["id"], json!(task_id.to_string()));
        assert_eq!(out[0]["evidence"]["command"], "cargo test");
        assert_eq!(out[0]["evidence"]["exit_code"], 0);
        assert_eq!(out[0]["evidence"]["artifact"], "run.log");
        assert!(out[0]["files"].is_array());
    }
}
