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
use sha2::{Digest, Sha256};
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
    let path = crate::graph::find_project_path(conn, project).ok()??;
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

/// Сводка улик задачи для списка (`task_list`/`au task list`): сколько
/// записей всего, сколько зелёных (`exit_code == 0`) и какая зелёная —
/// самая свежая. Полный массив с командами, временами и путями к
/// артефактам — по-прежнему только у `task_view`; сводка не заменяет его, а
/// экономит место там, где читателю нужно ответить «что есть и что
/// созрело», а не пересматривать журнал прогонов целиком (35 записей одной
/// задачи весили большую часть 20-тысячесимвольного `task_list` по 16
/// задачам, измерено 2026-09-05).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EvidenceSummary {
    pub total: usize,
    pub green: usize,
    pub last_green: Option<EvidenceEntry>,
}

/// Считает [`EvidenceSummary`] по уликам задачи — общая точка для CLI
/// (`au task list`) и MCP (`task_list`), чтобы правило «что зелёное» и «что
/// самое свежее» не разъезжалось между ними так же, как оно уже не
/// разъезжается у `ripe_evidence`.
///
/// При нескольких зелёных с одинаковым `at` побеждает более поздняя по
/// положению в массиве — `Iterator::max_by_key` возвращает последний из
/// равных, ту же гарантию использует `ripe_evidence` выше.
pub fn evidence_summary(fields: &TaskFields) -> EvidenceSummary {
    let total = fields.evidence.len();
    let green: Vec<&EvidenceEntry> = fields
        .evidence
        .iter()
        .filter(|e| e.exit_code == 0)
        .collect();
    let last_green = green.iter().max_by_key(|e| e.at).map(|e| (*e).clone());
    EvidenceSummary {
        total,
        green: green.len(),
        last_green,
    }
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

/// Key in `data` holding the met marks: handle -> RFC3339 timestamp of the
/// moment the criterion was first marked met.
///
/// Deliberately a sidecar. `data.acceptance_criteria` stays exactly what it
/// has always been — a bare JSON array of strings — because it is read by
/// [`crate::graph::task_acceptance_criteria`], by the MCP `task_show` and
/// `task_update` handlers, by the sync server and by the fitness gates, and
/// every one of those reads elements with `as_str()`. Turning a criterion
/// into an object would make it vanish from all of them at once: the fitness
/// gates would see a task with zero criteria and reclassify it. A separate
/// key adds state without a migration and without touching a single one of
/// the 407 task nodes already in the graph.
pub const CRITERIA_MET_KEY: &str = "criteria_met";

/// One acceptance criterion, with the handle it is addressed by.
///
/// The handle is derived from the criterion text, never from its position in
/// the list. Position is not a usable handle: inserting a criterion renumbers
/// every one after it, so a script or a later session marking "criterion 3"
/// marks a different sentence than the one it meant. A content-derived handle
/// is stable under insertion, removal and reordering of its neighbours, and
/// every task already in the graph gets its handles the moment it is read —
/// no migration, no sweep, no rewrite.
///
/// The price of deriving it from the text: editing a criterion's wording
/// gives it a new handle and orphans the old mark. That is the intended
/// reading — a criterion whose contract changed is no longer proven by the
/// evidence that met the old wording — and the orphan is reported by
/// `au task criterion` rather than silently dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct Criterion {
    /// Stable handle, eight lowercase hex characters.
    pub handle: String,
    /// The criterion text, trimmed, exactly as stored.
    pub text: String,
    /// When it was first marked met, or `None` if it is not met.
    pub met_at: Option<DateTime<Utc>>,
}

/// The stable handle of a criterion: the first four bytes of the SHA-256 of
/// its trimmed text, as eight lowercase hex characters.
///
/// Truncated on purpose — the handle is typed by a human at a terminal, and a
/// 64-character hash would not be. Eight hex characters is 2^32 of space
/// against a list that is realistically under twenty entries; the resolver
/// also accepts an unambiguous prefix, so in practice two or three characters
/// are typed. Two criteria with identical text share a handle, which is
/// correct: they are the same criterion written twice.
pub fn criterion_handle(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.trim().as_bytes());
    let digest = hasher.finalize();
    digest
        .iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

/// Reads a task's criteria with their handles and met state.
///
/// Acceptance of `data.acceptance_criteria` is deliberately identical to
/// [`crate::graph::task_acceptance_criteria`] — array of strings, trimmed,
/// empties dropped — so the two never disagree about what the criteria of a
/// task are. A task with no criteria, or with the key holding something other
/// than an array (four such nodes exist in the live graph, each holding one
/// multi-line string), yields an empty list here exactly as it does there.
pub fn task_criteria(data: &Value) -> Vec<Criterion> {
    let met = data.get(CRITERIA_MET_KEY).and_then(|v| v.as_object());
    data.get("acceptance_criteria")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|text| {
                    let handle = criterion_handle(text);
                    let met_at = met
                        .and_then(|m| m.get(&handle))
                        .and_then(|v| v.as_str())
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|t| t.with_timezone(&Utc));
                    Criterion {
                        handle,
                        text: text.to_owned(),
                        met_at,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Met marks whose criterion is no longer in the list — left behind when a
/// criterion's text was edited or the criterion was removed. Surfaced rather
/// than swept: a mark that quietly disappears looks like the command failed.
pub fn orphaned_criteria_marks(data: &Value) -> Vec<String> {
    let live: std::collections::HashSet<String> =
        task_criteria(data).into_iter().map(|c| c.handle).collect();
    data.get(CRITERIA_MET_KEY)
        .and_then(|v| v.as_object())
        .map(|m| {
            m.keys()
                .filter(|k| !live.contains(k.as_str()))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Resolves what the caller typed to exactly one criterion.
///
/// Accepted, in this order: `#N`, an explicit 1-based position; the exact
/// criterion text; an unambiguous prefix of a handle, the way a short git
/// hash works. Position sits behind the `#` on purpose — it stays reachable
/// for a human reading a numbered list, but nothing resolves to it by
/// accident, so a bare token is always the stable handle.
///
/// Every failure is an error, never a silent miss: an unknown handle marking
/// nothing would look identical to success.
pub fn resolve_criterion<'a>(
    criteria: &'a [Criterion],
    selector: &str,
) -> anyhow::Result<&'a Criterion> {
    let selector = selector.trim();
    if selector.is_empty() {
        anyhow::bail!("criterion selector is empty — pass a handle, the exact text, or #N");
    }
    if criteria.is_empty() {
        anyhow::bail!("task has no acceptance criteria to address");
    }

    if let Some(rest) = selector.strip_prefix('#') {
        let n: usize = rest.parse().map_err(|_| {
            anyhow::anyhow!("'#{rest}' is not a position — #N takes a 1-based number")
        })?;
        if n == 0 || n > criteria.len() {
            anyhow::bail!(
                "position #{n} is out of range — this task has {} acceptance criteria",
                criteria.len()
            );
        }
        return criteria.get(n - 1).ok_or_else(|| unreachable_position(n));
    }

    if let Some(exact) = criteria.iter().find(|c| c.text == selector) {
        return Ok(exact);
    }

    let lowered = selector.to_ascii_lowercase();
    if lowered.len() < 2 || !lowered.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!(
            "no acceptance criterion matches '{selector}' — pass a handle (at least two hex \
             characters), the exact criterion text, or #N. Handles: {}",
            handle_list(criteria)
        );
    }

    let matches: Vec<&Criterion> = criteria
        .iter()
        .filter(|c| c.handle.starts_with(&lowered))
        .collect();
    match matches.len() {
        1 => Ok(matches[0]),
        0 => anyhow::bail!(
            "no acceptance criterion has a handle starting with '{lowered}'. Handles: {}",
            handle_list(criteria)
        ),
        n => anyhow::bail!(
            "'{lowered}' matches {n} criteria — type more characters. Handles: {}",
            handle_list(criteria)
        ),
    }
}

fn handle_list(criteria: &[Criterion]) -> String {
    criteria
        .iter()
        .map(|c| c.handle.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn unreachable_position(n: usize) -> anyhow::Error {
    anyhow::anyhow!("position #{n} vanished between the range check and the read")
}

/// Writes the met mark of one criterion.
///
/// Targeted `json_set`/`json_remove` on the single path that changes, not the
/// read-modify-write of the whole `data` blob that `update_node` performs.
/// The reason is a real race, not tidiness: `au trace --hook` rewrites
/// `last_edit_at` on this same node on every file edit while the task is
/// active, and a whole-blob write from either side silently discards the
/// other. One path changed, one path written. Same reasoning as
/// `graph::set_fitness`.
///
/// Idempotent in both directions. Marking an already-met criterion keeps the
/// original timestamp rather than refreshing it — `met_at` records when the
/// criterion was first met, and a second call carries no new information.
/// Returns `true` if the stored state actually changed.
pub fn set_criterion_met(
    conn: &rusqlite::Connection,
    task_id: uuid::Uuid,
    handle: &str,
    met: bool,
) -> anyhow::Result<bool> {
    // The handle is concatenated into a JSON path, so it is checked here
    // rather than trusted: everything that produces one is `criterion_handle`,
    // which emits hex only, and anything else is a bug upstream.
    if handle.is_empty() || !handle.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("criterion handle must be hex characters, got '{handle}'");
    }

    let id_str = task_id.to_string();
    let node = crate::graph::get_node(conn, &id_str)?
        .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;
    let already = task_criteria(&node.data)
        .into_iter()
        .any(|c| c.handle == handle && c.met_at.is_some());
    if already == met {
        return Ok(false);
    }

    let now = Utc::now();
    let author = crate::identity::current().map(|i| i.as_author());
    let affected = if met {
        conn.execute(
            SET_CRITERION_MET_SQL,
            rusqlite::params![handle, now.to_rfc3339(), now.to_rfc3339(), author, id_str],
        )?
    } else {
        conn.execute(
            CLEAR_CRITERION_MET_SQL,
            rusqlite::params![handle, now.to_rfc3339(), author, id_str],
        )?
    };
    if affected == 0 {
        anyhow::bail!("task not found or deleted: {task_id}");
    }
    crate::db::mark_write(conn);
    Ok(true)
}

/// Creates `data.criteria_met` if it is absent before writing into it —
/// `json_set` on a nested path whose parent does not exist is a no-op, and no
/// task in the graph has the key yet.
const SET_CRITERION_MET_SQL: &str = "
UPDATE nodes SET
  data = json_set(
    json_set(data, '$.criteria_met', json(coalesce(json_extract(data, '$.criteria_met'), '{}'))),
    '$.criteria_met.\"' || ?1 || '\"',
    ?2
  ),
  updated_at = ?3, updated_by = ?4
WHERE id = ?5 AND deleted_at IS NULL AND node_type = '\"task\"'
";

const CLEAR_CRITERION_MET_SQL: &str = "
UPDATE nodes SET
  data = json_remove(data, '$.criteria_met.\"' || ?1 || '\"'),
  updated_at = ?2, updated_by = ?3
WHERE id = ?4 AND deleted_at IS NULL AND node_type = '\"task\"'
";

/// First `max` characters of `text`, cut on a character boundary so a
/// multi-byte character never gets split in half.
fn truncate_label(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Builds the work-log node for a task and wires it into the graph — the one
/// place both `au task log` (CLI) and MCP `task_log` build a work-log node,
/// so the two stop keeping their own copies of the same label/note/source/
/// edges construction.
///
/// Recording an observation is a different action from taking a task into
/// work, so this function never touches `task.data["status"]`: the caller
/// passes `data` with `task_id` (and provenance, if any) already written
/// into it, and activation goes through `activate_task` (CLI) or
/// `task_update status=active` (MCP) only — see the task-log-activation
/// decision in project memory.
pub fn log_work(
    conn: &rusqlite::Connection,
    task: &crate::models::Node,
    text: &str,
    source: &str,
    data: Value,
) -> anyhow::Result<crate::models::Node> {
    let project = task
        .data
        .get("project")
        .and_then(|p| p.as_str())
        .unwrap_or("unknown");
    let label = format!("[{project}] {}", truncate_label(text, 60));

    let log_node = crate::graph::add_node_full(
        conn,
        crate::models::NodeType::WorkLog,
        &label,
        Some(text),
        source,
        data,
        crate::models::MemoryKind::Episodic,
        None,
    )?;

    crate::graph::add_edge(
        conn,
        task.id,
        log_node.id,
        crate::models::Relation::Contains,
        1.0,
    )?;

    if let Ok(Some(proj_node)) = crate::graph::find_project_by_label(conn, project) {
        crate::graph::add_edge(
            conn,
            log_node.id,
            proj_node.id,
            crate::models::Relation::BelongsTo,
            1.0,
        )?;
    }

    Ok(log_node)
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

    /// Три улики (красная, зелёная, зелёная позже первой) дают сводку 3/2,
    /// а `last_green` — САМУЮ СВЕЖУЮ зелёную, а не первую попавшуюся.
    #[test]
    fn evidence_summary_counts_green_and_picks_latest() {
        let fields = TaskFields {
            evidence: vec![
                evidence("cargo test", 1, "2026-08-30T09:00:00Z"),
                evidence("cargo clippy", 0, "2026-08-30T09:30:00Z"),
                evidence("cargo test", 0, "2026-08-30T10:00:00Z"),
            ],
            ..Default::default()
        };

        let summary = evidence_summary(&fields);

        assert_eq!(summary.total, 3);
        assert_eq!(summary.green, 2);
        assert_eq!(
            summary.last_green.expect("зелёная улика есть").command,
            "cargo test"
        );
    }

    /// Пустой список улик — 0/0/None, а не паника на пустом `max_by_key`.
    #[test]
    fn evidence_summary_of_empty_list_is_zero_and_none() {
        let summary = evidence_summary(&TaskFields::default());

        assert_eq!(summary.total, 0);
        assert_eq!(summary.green, 0);
        assert!(summary.last_green.is_none());
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

    /// В живой базе узлов проекта с одной меткой несколько: свой заводит
    /// индексатор, свой — автосоздание при заведении задачи, и путь записан
    /// не у каждого. Каталог надо искать среди тех, где он есть, иначе он
    /// находится через раз — по порядку строк, — а от него зависит, соберутся
    /// ли файлы в способ решения.
    #[test]
    fn project_root_skips_project_node_without_path() {
        let (_tmp, conn) = setup();
        crate::graph::add_node(
            &conn,
            crate::models::NodeType::Project,
            "proj-twice",
            None,
            "test",
            json!({}),
        )
        .expect("insert project without path");
        seed_project_with_path(&conn, "proj-twice", "/repos/proj-twice");

        let root = project_root(&conn, "proj-twice");

        assert_eq!(root, Some(PathBuf::from("/repos/proj-twice")));
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

    // -- task 311cae6a: `log_work` is the one function both `au task log`
    // (CLI) and MCP `task_log` call — the CLI match arm itself is inline in
    // `pub async fn task`, bound to the real `db_path()`, so it is not
    // testable without spawning a process; this exercises the shared helper
    // directly instead, which is what the CLI arm now delegates to.

    /// `log_work` records an observation, not a decision to take the task
    /// into work: it must never touch `data.status` or stamp `activated_at`,
    /// no matter what status the task was in.
    #[test]
    fn log_work_does_not_change_task_status() {
        let (_tmp, conn) = setup();
        let task_id = seed_task(
            &conn,
            "задача в очереди",
            json!({"status": "backlog", "priority": "medium", "project": "proj-log-work"}),
        );
        let task = crate::graph::get_node(&conn, &task_id.to_string())
            .expect("get_node")
            .expect("task exists");

        log_work(
            &conn,
            &task,
            "текст записи",
            "test",
            json!({"task_id": task_id.to_string()}),
        )
        .expect("log_work");

        let after = crate::graph::get_node(&conn, &task_id.to_string())
            .expect("get_node")
            .expect("task still exists");
        assert_eq!(
            after.data.get("status").and_then(|s| s.as_str()),
            Some("backlog"),
            "log_work must not activate the task"
        );
        assert!(
            after.data.get("activated_at").is_none(),
            "log_work must not stamp activated_at"
        );
    }

    /// The work-log node it creates is wired to the task with a `contains`
    /// edge — both callers relied on this edge existing before the two
    /// inline copies were replaced by this one function.
    #[test]
    fn log_work_wires_contains_edge_from_task() {
        let (_tmp, conn) = setup();
        let task_id = seed_task(
            &conn,
            "задача под ребро",
            json!({"status": "active", "priority": "medium", "project": "proj-log-work-edge"}),
        );
        let task = crate::graph::get_node(&conn, &task_id.to_string())
            .expect("get_node")
            .expect("task exists");

        let log_node = log_work(
            &conn,
            &task,
            "текст записи",
            "test",
            json!({"task_id": task_id.to_string()}),
        )
        .expect("log_work");

        let edge = crate::graph::find_edge(
            &conn,
            task_id,
            log_node.id,
            &crate::models::Relation::Contains,
        )
        .expect("find_edge")
        .expect("task --contains--> worklog edge must exist");
        assert_eq!(edge.from_id, task_id);
        assert_eq!(edge.to_id, log_node.id);
    }
}
