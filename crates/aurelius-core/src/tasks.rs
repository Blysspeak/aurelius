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
use std::path::{Path, PathBuf};

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
    ///
    /// Каждое поле читается по отдельности, а не структура целиком: разбор
    /// разом (`from_value::<Self>(data).unwrap_or_default()`) означал, что
    /// одно испорченное значение где угодно в `data` молча обнуляет ВСЕ
    /// поля сразу. Задача с настоящими правкой и зелёной уликой переставала
    /// считаться созревшей из-за, например, числа вместо строки в
    /// `resolution.files` — без единого сообщения о том, что что-то не так.
    /// Порча остаётся локальной: теряется только то поле, которое испорчено.
    pub fn from_data(data: &Value) -> Self {
        Self {
            activated_at: field(data, "activated_at"),
            closed_at: field(data, "closed_at"),
            resolution: field(data, "resolution"),
            evidence: readable_evidence(data),
            last_edit_at: field(data, "last_edit_at"),
            declined_ripe_at: field(data, "declined_ripe_at"),
        }
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

/// Одно поле из `data`, если оно там есть и читается. Нечитаемое поле —
/// `None`: оно теряется в одиночку, не утаскивая за собой соседей.
fn field<T: serde::de::DeserializeOwned>(data: &Value, key: &str) -> Option<T> {
    serde_json::from_value(data.get(key)?.clone()).ok()
}

/// Улики, которые удалось прочесть. Разбор поэлементный, а не разбор всего
/// массива разом: одна улика с испорченной датой обнуляла бы весь список,
/// включая прогоны, записанные правильно.
fn readable_evidence(data: &Value) -> Vec<EvidenceEntry> {
    data.get("evidence")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Корневой каталог проекта по его имени (`data.project` задачи).
///
/// Узел проекта в графе хранит канонический путь в `data.path` — его пишет
/// индексатор (`indexer.rs::get_or_create_project`) при первой индексации.
/// `None` значит «неизвестно», а не «текущий каталог»: проекта с такой
/// меткой нет, либо узел заведён не индексатором и пути не знает (например,
/// `task_create` создаёт узел проекта на лету, без `data.path`, если проект
/// раньше не индексировался). Вызывающий обязан читать `None` как «каталог
/// этой задачи неизвестен», а не подставлять вместо него CWD процесса —
/// именно эта подмена и была находкой 1 (Aurelius — один процесс на все
/// проекты, CWD принадлежит тому, где его запустили, а не задаче).
pub fn project_root(conn: &rusqlite::Connection, project: &str) -> Option<PathBuf> {
    let node = crate::graph::find_project_by_label(conn, project).ok()??;
    let path = node.data.get("path")?.as_str()?;
    Some(PathBuf::from(path))
}

/// Коммит, которым решается задача, если способ решения не назвали явно
/// (T021a, FR-006): `git rev-parse --short HEAD`. `dir` — каталог, в котором
/// искать репозиторий (`git -C <dir>`); `None` — команда идёт в текущем
/// рабочем каталоге процесса, что годится только вызывающему, у которого нет
/// понятия «каталог задачи» вовсе (см. `build_resolution`). `None` в
/// результате — не повод отказать в закрытии, только не сможем назвать
/// коммит: не git-репозиторий, каталог не существует, команда недоступна.
///
/// Общая точка для CLI (`au task done`) и MCP (`task_update`, статус `done`)
/// — то же самое правило, вызванное из обоих мест, а не продублированное.
pub fn current_commit_sha(dir: Option<&Path>) -> Option<String> {
    let mut cmd = std::process::Command::new("git");
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
    }
    cmd.args(["rev-parse", "--short", "HEAD"]);
    let output = cmd.output().ok()?;
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
/// `project` — проект ЗАКРЫВАЕМОЙ задачи, не каталог процесса (находка 1
/// адверсариального разбора спеки 007): Aurelius — один процесс и одна БД на
/// все проекты, поэтому CWD процесса — не то же самое, что каталог проекта
/// задачи. Автоподстановка коммита смотрит в каталог именно этого проекта
/// (`project_root`), а не в CWD:
/// - проект назван и его каталог известен — коммит берётся из НЕГО;
/// - проект назван, а каталог неизвестен — коммит не подставляется вовсе:
///   пустой способ решения честнее ложного с `confirmed: true`;
/// - проект не назван совсем (старый однопроектный вызов) — CWD процесса,
///   как и раньше: единственный источник, который у такого вызова был.
///
/// Список файлов (находка 2) ограничен тем же каталогом — см.
/// `crate::trace::files_edited_since`.
///
/// Общая точка для CLI (`au task done`) и MCP (`task_update`, статус `done`).
pub fn build_resolution(
    conn: &rusqlite::Connection,
    since: DateTime<Utc>,
    project: Option<&str>,
    commit: Option<String>,
    pull_request: Option<String>,
    unconfirmed: bool,
) -> Resolution {
    let root = project.and_then(|p| project_root(conn, p));
    let commit = commit.or_else(|| match project {
        Some(_) => root
            .as_deref()
            .and_then(|dir| current_commit_sha(Some(dir))),
        None => current_commit_sha(None),
    });
    let files = crate::trace::files_edited_since(conn, since.timestamp(), root.as_deref())
        .unwrap_or_default();
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
/// все разом:
/// 1. `status == "active"`;
/// 2. есть `last_edit_at`;
/// 3. в `evidence` есть элемент с `exit_code == 0` и `at` позже `last_edit_at`;
/// 4. `declined_ripe_at` отсутствует или старше `last_edit_at`;
/// 5. если есть `activated_at` — `last_edit_at` строго позже него (созревание
///    считается по ТЕКУЩЕМУ циклу работы, не по прошлому).
///
/// Условие 5 — починка находки 3 (адверсариальный разбор спеки 007):
/// переоткрытие (`done` → `active`) обновляет только `activated_at` —
/// `last_edit_at`, `evidence` и `declined_ripe_at` от прошлого цикла
/// остаются как есть (FR-003 прямо требует, чтобы переоткрытие их не
/// стирало, спека 007 §data-model.md). Без условия 5 задача с зелёной уликой
/// прошлого цикла считалась бы созревшей СРАЗУ после переоткрытия — без
/// единой новой правки и прогона в новом цикле. Раз `last_edit_at` обязан
/// быть позже `activated_at`, а подходящая улика (условие 3) обязана быть
/// позже `last_edit_at` — она тем самым тоже гарантированно позже
/// `activated_at`, отдельная проверка по улике не нужна.
/// Задачи без `activated_at` (заведены до этой фичи, миграция назад не
/// делалась) условие 5 не проверяет вовсе — иначе они разом перестали бы
/// созревать, хотя раньше созревали.
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
    if let Some(activated_at) = fields.activated_at {
        if last_edit_at <= activated_at {
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
        // Находка 2: границу проекта задаёт проект ЭТОЙ задачи (`t.data`), а
        // не параметр `project` вызова — тот лишь фильтр выборки и может
        // быть `None` (все проекты разом).
        let task_project = t.data.get("project").and_then(|p| p.as_str());
        let root = task_project.and_then(|p| project_root(conn, p));
        let files = crate::trace::files_edited_since(conn, since.timestamp(), root.as_deref())
            .unwrap_or_default();
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

    /// Одно испорченное значение не должно уносить с собой соседние поля.
    /// Разбор всей структуры разом делал именно это: число вместо строки в
    /// `resolution.files` обнуляло и времена, и улики, и задача с настоящей
    /// правкой и зелёным прогоном молча переставала считаться созревшей.
    #[test]
    fn corrupt_field_does_not_wipe_the_readable_ones() {
        let data = json!({
            "status": "active",
            "activated_at": "2026-08-30T08:00:00Z",
            "last_edit_at": "2026-08-30T09:00:00Z",
            "evidence": [
                {"command": "cargo test", "exit_code": 0, "at": "2026-08-30T10:00:00Z"},
            ],
            "resolution": {"files": [123]},
        });

        let fields = TaskFields::from_data(&data);

        assert_eq!(fields.evidence.len(), 1, "улика читается: {fields:?}");
        assert!(fields.last_edit_at.is_some(), "время правки читается");
        assert!(
            fields.resolution.is_none(),
            "испорченный способ решения теряется — но только он"
        );
        assert!(
            is_ripe(&fields, "active"),
            "созревание считается по читаемым полям, а не отменяется чужой порчей"
        );
    }

    /// Улики разбираются поэлементно: соседство с испорченной записью не
    /// должно стоить прогону места в списке.
    #[test]
    fn corrupt_evidence_entry_does_not_hide_the_others() {
        let data = json!({
            "evidence": [
                {"command": "cargo test", "exit_code": 0, "at": "не дата"},
                {"command": "cargo clippy", "exit_code": 0, "at": "2026-08-30T10:00:00Z"},
            ],
        });

        let fields = TaskFields::from_data(&data);

        assert_eq!(fields.evidence.len(), 1, "читаемая улика остаётся");
        assert_eq!(fields.evidence[0].command, "cargo clippy");
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

    /// Находка 3 (адверсариальный разбор спеки 007): переоткрытие
    /// (`done` → `active`) обновляет только `activated_at` — `last_edit_at`
    /// и зелёная улика от ПРОШЛОГО цикла остаются нетронутыми (FR-003
    /// запрещает их стирать). Без условия 5 в `ripe_evidence` задача была бы
    /// созревшей сразу после переоткрытия, без единой новой правки. Тест
    /// падал на прежней реализации `ripe_evidence` (проверявшей только
    /// `last_edit_at`/`evidence`/`declined_ripe_at`, без сравнения с
    /// `activated_at`).
    #[test]
    fn reopened_task_is_not_immediately_ripe_from_previous_cycle() {
        let fields = TaskFields {
            // Прошлый цикл: правка и зелёная улика — обе ДО переоткрытия.
            last_edit_at: Some("2026-08-30T09:00:00Z".parse().expect("rfc3339")),
            evidence: vec![evidence("cargo test", 0, "2026-08-30T09:30:00Z")],
            // Переоткрытие — позже и правки, и улики прошлого цикла.
            activated_at: Some("2026-08-30T12:00:00Z".parse().expect("rfc3339")),
            ..Default::default()
        };

        assert!(
            !is_ripe(&fields, "active"),
            "улика прошлого цикла не обязана созревать задачу после переоткрытия"
        );
    }

    /// Асимметрия предыдущего теста: НОВАЯ правка и НОВАЯ улика ПОСЛЕ
    /// переоткрытия — созревание обязано вернуться, условие 5 не должно
    /// блокировать текущий цикл навечно.
    #[test]
    fn reopened_task_becomes_ripe_again_after_new_cycle_evidence() {
        let fields = TaskFields {
            last_edit_at: Some("2026-08-30T13:00:00Z".parse().expect("rfc3339")),
            evidence: vec![evidence("cargo test", 0, "2026-08-30T13:30:00Z")],
            activated_at: Some("2026-08-30T12:00:00Z".parse().expect("rfc3339")),
            ..Default::default()
        };

        assert!(is_ripe(&fields, "active"));
    }

    /// Задача без `activated_at` вовсе (заведена до этой фичи) не обязана
    /// проверяться условием 5 — иначе она разом перестала бы созревать,
    /// хотя раньше созревала. Тот же случай, что и `not_ripe_without_last_edit_at`
    /// выше, но с непустой уликой — подтверждает, что отсутствие
    /// `activated_at` не блокирует созревание.
    #[test]
    fn ripe_without_activated_at_is_unaffected_by_cycle_check() {
        let fields = TaskFields {
            last_edit_at: Some("2026-08-30T09:00:00Z".parse().expect("rfc3339")),
            evidence: vec![evidence("cargo test", 0, "2026-08-30T10:00:00Z")],
            ..Default::default()
        };

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

    /// Узел проекта с `data.path` — то, что реально пишет индексатор
    /// (`indexer.rs::get_or_create_project`). `find_project_by_label` ищет по
    /// метке И типу узла — `add_node` типа `Project` этого достаточно.
    fn seed_project_with_path(conn: &rusqlite::Connection, label: &str, path: &str) {
        crate::graph::add_node(
            conn,
            crate::models::NodeType::Project,
            label,
            None,
            "test",
            json!({"path": path}),
        )
        .expect("insert project");
    }

    // -- project_root / build_resolution: находка 1 -------------------------

    #[test]
    fn project_root_reads_path_from_project_node() {
        let (_tmp, conn) = setup();
        seed_project_with_path(&conn, "proj-with-path", "/repos/proj-with-path");

        let root = project_root(&conn, "proj-with-path");

        assert_eq!(root, Some(PathBuf::from("/repos/proj-with-path")));
    }

    #[test]
    fn project_root_is_none_for_unknown_project() {
        let (_tmp, conn) = setup();

        assert_eq!(project_root(&conn, "нет такого проекта"), None);
    }

    /// Находка 1 (адверсариальный разбор спеки 007): проект задачи назван, но
    /// его каталог неизвестен графу (узел проекта не индексирован либо не
    /// хранит `data.path`) — коммит НЕ подставляется автоматически, даже
    /// если процесс сам работает внутри какого-то git-репозитория (CWD теста
    /// — рабочее дерево aurelius, оно ГАРАНТИРОВАННО git-репозиторий). Тест
    /// падал на прежней реализации (`current_commit_sha()` без аргументов,
    /// бравшей коммит из CWD процесса вне зависимости от того, чей это
    /// проект) и проходит на новой.
    #[test]
    fn build_resolution_does_not_guess_commit_when_project_root_unknown() {
        let (_tmp, conn) = setup();
        let since = "2020-01-01T00:00:00Z".parse().expect("rfc3339");

        let resolution = build_resolution(
            &conn,
            since,
            Some("проект-без-индексации"),
            None,
            None,
            false,
        );

        assert_eq!(
            resolution.commit, None,
            "пустой коммит честнее подставленного из CWD чужого проекта"
        );
    }

    /// Прямая репродукция находки 1: два РЕАЛЬНЫХ git-репозитория. Процесс
    /// (тест) работает в каталоге A (workspace aurelius), а задача
    /// принадлежит проекту B — отдельному репозиторию во временном каталоге.
    /// `build_resolution` обязана вернуть SHA репозитория B, а не текущего
    /// каталога процесса.
    #[test]
    fn build_resolution_uses_task_project_repo_not_process_cwd() {
        let (_tmp, conn) = setup();

        let repo_b =
            std::env::temp_dir().join(format!("aurelius-tasks-repo-b-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&repo_b).expect("mkdir repo_b");
        let run_git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo_b)
                .args(args)
                .output()
                .expect("запустить git")
        };
        assert!(run_git(&["init", "-q"]).status.success());
        assert!(run_git(&["config", "user.email", "test@example.com"])
            .status
            .success());
        assert!(run_git(&["config", "user.name", "test"]).status.success());
        std::fs::write(repo_b.join("README.md"), "b").expect("write file");
        assert!(run_git(&["add", "."]).status.success());
        assert!(run_git(&["commit", "-q", "-m", "init"]).status.success());
        let expected_sha =
            String::from_utf8_lossy(&run_git(&["rev-parse", "--short", "HEAD"]).stdout)
                .trim()
                .to_owned();

        seed_project_with_path(&conn, "proj-b", &repo_b.to_string_lossy());
        let since = "2020-01-01T00:00:00Z".parse().expect("rfc3339");

        let resolution = build_resolution(&conn, since, Some("proj-b"), None, None, false);

        assert_eq!(resolution.commit.as_deref(), Some(expected_sha.as_str()));
        // Не CWD процесса: каталог теста (`aurelius`) — другой репозиторий с
        // другой историей, совпадение SHA было бы подозрительным само по себе.
        assert_ne!(resolution.commit, current_commit_sha(None));

        std::fs::remove_dir_all(&repo_b).ok();
    }

    // -- gather_ripe: находка 2 — файлы ограничены проектом задачи ----------

    /// Находка 2 (адверсариальный разбор спеки 007): без границы проекта
    /// хук `au trace --hook` пишет правки ЛЮБОГО проекта в общую таблицу
    /// `act_trace` — список файлов созревшей задачи проекта A не обязан
    /// содержать правки проекта B. Тест падал на прежней реализации
    /// (`files_edited_since` без параметра каталога, фильтровавшей только по
    /// времени) и проходит на новой.
    #[test]
    fn gather_ripe_files_are_scoped_to_the_task_own_project() {
        let (_tmp, conn) = setup();
        seed_project_with_path(&conn, "proj-a", "/repos/proj-a");
        let task_id = seed_task(
            &conn,
            "[proj-a] созревшая задача",
            json!({
                "status": "active",
                "priority": "medium",
                "project": "proj-a",
                "last_edit_at": "2026-08-30T09:00:00Z",
                "evidence": [{
                    "command": "cargo test",
                    "exit_code": 0,
                    "at": "2026-08-30T10:00:00Z",
                }],
            }),
        );
        for payload in ["/repos/proj-a/src/lib.rs", "/repos/proj-b/src/lib.rs"] {
            crate::trace::ingest(
                &conn,
                &crate::trace::TraceInput {
                    session_id: "s1",
                    kind: crate::trace::TraceKind::FileEdit,
                    payload,
                    exit_code: None,
                    state_hash_pre: None,
                    state_hash_post: None,
                },
            )
            .expect("ingest trace");
        }

        let ripe = gather_ripe(&conn, None).expect("gather_ripe");

        assert_eq!(ripe.len(), 1);
        assert_eq!(ripe[0].id, task_id);
        assert_eq!(ripe[0].files, vec!["/repos/proj-a/src/lib.rs".to_owned()]);
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
