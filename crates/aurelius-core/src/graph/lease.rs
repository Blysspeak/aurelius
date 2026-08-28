//! Наряд: аренда задачи для автономного исполнителя (спека 006, фаза 2).
//!
//! Задача уже хранится узлом графа с произвольным JSON в `data` — наряд
//! добавляется ключами `lease`, `attempts`, `cooldown_until`, `done_by`, без
//! миграции схемы. Старые задачи без этих ключей читаются через `COALESCE` в
//! каждом запросе.
//!
//! Одна деталь, стоившая бы дня отладки: `node_type` хранится **с кавычками**
//! (`serde_json::to_string(&NodeType::Task)` даёт `"task"`), поэтому во всех
//! запросах ниже сравнение идёт с литералом `'"task"'`, а не `'task'`.
//!
//! Фаза 3 (спека 006) добавляет сюда же разметку исполнимости: гейты отсева,
//! перенесённые из одноразового скрипта `fitness-dryrun.mjs` (research.md,
//! «Волна 0: результат»), и запись вердикта в `data.fitness`.

use chrono::{DateTime, Utc};
use regex::Regex;
use rusqlite::{params, Connection, ErrorCode};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use thiserror::Error;
use uuid::Uuid;

/// Сколько раз задачу можно взять в работу, прежде чем она блокируется сама.
/// Должно совпадать с литералом `< 3` в [`CLAIM_SQL`] — это отдельные места
/// по необходимости (SQL сравнивает до инкремента, Rust — после), а не по
/// небрежности.
const MAX_ATTEMPTS: i64 = 3;

/// Фиксированное остывание после неудачи. Числа потолков в спеке (research.md)
/// названы взятыми по здравому смыслу, а не измеренными — это одно из них.
const DEFAULT_COOLDOWN_MINUTES: i64 = 5;

/// Ошибки взятия/продления/отпускания наряда, различимые кодом возврата CLI
/// (контракт `au-task-cli.md`), а не текстом сообщения — принцип III
/// конституции запрещает классифицировать ошибки по человекочитаемой строке.
#[derive(Debug, Error)]
pub enum LeaseError {
    /// Пул пуст: нет задачи с вердиктом `machine`, ушедшей на волю и с
    /// прошедшим остыванием. Не ошибка вызова — коду CLI это код `10`.
    #[error("свободных нарядов нет")]
    NoTasksAvailable,
    /// `renew`/`release`/`give-up` на наряд, которым владеет не этот
    /// вызывающий: либо аренда истекла и перевыдана, либо id не существует.
    /// Оба случая контракт сводит к одному коду (`10`) — обманывать вызывающего
    /// поддельным успехом хуже, чем не различать их источник.
    #[error("наряд не принадлежит этому владельцу")]
    NotOwner,
    /// SQLite вернул `SQLITE_BUSY`/`SQLITE_LOCKED` — классифицировано по коду
    /// ошибки (`rusqlite::ErrorCode`), не по тексту. FR-010: это MUST NOT
    /// читаться как «нарядов нет», иначе ручная сессия владельца выглядит как
    /// опустевшая очередь.
    #[error("база занята другим писателем")]
    Busy(#[source] rusqlite::Error),
}

/// Наряд, отданный вызывающему: то, что `RETURNING` фактически возвращает —
/// не весь узел (лишние столбцы никому здесь не нужны), а то, из чего
/// собирается промпт звена.
#[derive(Debug, Clone)]
pub struct ClaimedTask {
    pub id: Uuid,
    pub label: String,
    pub note: Option<String>,
    pub data: Value,
}

/// Заявленный исход работы над нарядом.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Done,
    Failed,
}

/// Итог `release`: что реально записано в задачу.
#[derive(Debug, Clone)]
pub struct ReleaseOutcome {
    /// `done` | `backlog` | `blocked`
    pub status: String,
    /// Причина блокировки — есть только когда `status == "blocked"`.
    pub reason: Option<String>,
}

/// Взятие наряда — SQL из research.md, «Компонент 1. Наряд». Условия отбора и
/// сама запись — без изменений; единственное добавление — необязательный
/// фильтр по проекту (`?5`), которого research.md не описывает, но контракт
/// `au-task-cli.md` перечисляет как `[--project <имя>]`. `?5 IS NULL` делает
/// условие сквозным, когда фильтр не задан, поэтому поведение без `--project`
/// побайтово то же, что в research.md.
///
/// Одно предложение `UPDATE ... RETURNING`: подзапрос выбора и запись в
/// одной атомарной операции, поэтому второй писатель либо ждёт по
/// `busy_timeout` (заданному `db::open`), либо видит уже занятый наряд с
/// живой арендой — двойного взятия не бывает (FR-006).
const CLAIM_SQL: &str = "
UPDATE nodes SET
  data = json_set(json_set(json_set(data,
           '$.status','active'),
           '$.lease', json_object('owner',?1,'until',?2,'run',?3)),
           '$.attempts', COALESCE(json_extract(data,'$.attempts'),0)+1),
  updated_at = ?4, updated_by = ?1
WHERE id = (
  SELECT id FROM nodes
   WHERE node_type = '\"task\"' AND deleted_at IS NULL      -- кавычки обязательны
     AND json_extract(data,'$.fitness.verdict') = 'machine'
     AND COALESCE(json_extract(data,'$.attempts'),0) < 3
     AND COALESCE(json_extract(data,'$.cooldown_until'),'') < ?4
     AND ( json_extract(data,'$.status') = 'backlog'
        OR ( json_extract(data,'$.status') = 'active'     -- брошенный наряд
             AND COALESCE(json_extract(data,'$.lease.until'),'') < ?4 ) )
     AND ( ?5 IS NULL OR json_extract(data,'$.project') = ?5 )
   ORDER BY CASE json_extract(data,'$.priority')
              WHEN 'critical' THEN 0 WHEN 'high' THEN 1
              WHEN 'medium' THEN 2 ELSE 3 END,
            COALESCE(json_extract(data,'$.attempts'),0),
            created_at
   LIMIT 1)
RETURNING id, label, note, data;
";

/// Продлевает аренду. Пишет только владелец, указанный в `data.lease.owner`
/// на момент вызова — чужой наряд получает отказ (`NotOwner`), не молчаливый
/// успех (FR-009).
const RENEW_SQL: &str = "
UPDATE nodes SET
  data = json_set(data, '$.lease.until', ?1),
  updated_at = ?2, updated_by = ?3
WHERE id = ?4 AND deleted_at IS NULL AND node_type = '\"task\"'
  AND json_extract(data,'$.lease.owner') = ?3
";

const RELEASE_DONE_SQL: &str = "
UPDATE nodes SET
  data = json_set(json_set(data, '$.status', 'done'), '$.done_by', 'smena'),
  updated_at = ?1, updated_by = ?2
WHERE id = ?3 AND deleted_at IS NULL AND node_type = '\"task\"'
  AND json_extract(data,'$.lease.owner') = ?2
";

const RELEASE_RETRY_SQL: &str = "
UPDATE nodes SET
  data = json_set(json_set(data, '$.status', 'backlog'), '$.cooldown_until', ?1),
  updated_at = ?2, updated_by = ?3
WHERE id = ?4 AND deleted_at IS NULL AND node_type = '\"task\"'
  AND json_extract(data,'$.lease.owner') = ?3
";

const RELEASE_BLOCK_SQL: &str = "
UPDATE nodes SET
  data = json_set(json_set(data, '$.status', 'blocked'), '$.blocked_by', ?1),
  updated_at = ?2, updated_by = ?3
WHERE id = ?4 AND deleted_at IS NULL AND node_type = '\"task\"'
  AND json_extract(data,'$.lease.owner') = ?3
";

const GIVE_UP_SQL: &str = "
UPDATE nodes SET
  data = json_set(json_set(data, '$.status', 'blocked'), '$.blocked_by', ?1),
  updated_at = ?2, updated_by = ?3
WHERE id = ?4 AND deleted_at IS NULL AND node_type = '\"task\"'
  AND json_extract(data,'$.lease.owner') = ?3
";

/// `true`, если SQLite отказал занятостью (`SQLITE_BUSY`/`SQLITE_LOCKED`), а
/// не чем-то иным. Смотрит на `rusqlite::ErrorCode`, никогда на текст ошибки.
fn is_busy(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(ffi_err, _)
            if matches!(ffi_err.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

/// Заворачивает сырую ошибку `rusqlite` в типизированную `LeaseError::Busy`,
/// когда это занятость; иначе пропускает как есть — такая ошибка не наша,
/// она падает в общую классификацию хранилища на границе CLI.
fn map_sqlite_err(err: rusqlite::Error) -> anyhow::Error {
    if is_busy(&err) {
        LeaseError::Busy(err).into()
    } else {
        err.into()
    }
}

/// Взять один наряд из машинного пула. `project` сужает отбор до задач с тем
/// же `data.project`; `None` не сужает ничего. См. [`CLAIM_SQL`].
pub fn claim(
    conn: &Connection,
    owner: &str,
    run: &str,
    lease_minutes: i64,
    project: Option<&str>,
) -> anyhow::Result<ClaimedTask> {
    let now = Utc::now();
    let until = now + chrono::Duration::minutes(lease_minutes);
    let now_str = now.to_rfc3339();
    let until_str = until.to_rfc3339();

    let mut stmt = conn.prepare(CLAIM_SQL).map_err(map_sqlite_err)?;
    let row = stmt.query_row(params![owner, until_str, run, now_str, project], |row| {
        let id: String = row.get(0)?;
        let label: String = row.get(1)?;
        let note: Option<String> = row.get(2)?;
        let data: String = row.get(3)?;
        Ok((id, label, note, data))
    });

    let (id, label, note, data) = match row {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return Err(LeaseError::NoTasksAvailable.into())
        }
        Err(e) => return Err(map_sqlite_err(e)),
    };

    let id = id
        .parse::<Uuid>()
        .map_err(|e| anyhow::anyhow!("наряд вернул нечитаемый id '{id}': {e}"))?;
    let data: Value = serde_json::from_str(&data)
        .map_err(|e| anyhow::anyhow!("наряд вернул нечитаемый data: {e}"))?;

    Ok(ClaimedTask {
        id,
        label,
        note,
        data,
    })
}

/// Продлить аренду взятого наряда. Вызывает смена, пока дочерний процесс
/// жив, — не исполнитель: у смены есть handle процесса, у модели только
/// собственное мнение о том, что она жива (FR-009).
pub fn renew(
    conn: &Connection,
    id: Uuid,
    owner: &str,
    lease_minutes: i64,
) -> anyhow::Result<DateTime<Utc>> {
    let now = Utc::now();
    let until = now + chrono::Duration::minutes(lease_minutes);
    let affected = conn
        .execute(
            RENEW_SQL,
            params![until.to_rfc3339(), now.to_rfc3339(), owner, id.to_string()],
        )
        .map_err(map_sqlite_err)?;
    if affected == 0 {
        return Err(LeaseError::NotOwner.into());
    }
    Ok(until)
}

/// Заявить исход работы. `Verdict::Done` закрывает задачу (`done_by=smena`);
/// `Verdict::Failed` возвращает её в очередь с остыванием — если это была уже
/// третья попытка, вместо возврата задача блокируется с причиной, где указано
/// число попыток (FR-008): отбор `attempts < 3` в [`CLAIM_SQL`] сам по себе
/// лишь перестаёт её выдавать, а не помечает, и молча пропавшая из очереди
/// задача утром выглядит забытой, а не исчерпанной.
pub fn release(
    conn: &Connection,
    id: Uuid,
    owner: &str,
    verdict: Verdict,
    evidence: &str,
) -> anyhow::Result<ReleaseOutcome> {
    if evidence.trim().is_empty() {
        anyhow::bail!("--evidence не может быть пустым");
    }
    match verdict {
        Verdict::Done => release_done(conn, id, owner),
        Verdict::Failed => release_failed(conn, id, owner),
    }
}

fn release_done(conn: &Connection, id: Uuid, owner: &str) -> anyhow::Result<ReleaseOutcome> {
    let now = Utc::now();
    let affected = conn
        .execute(
            RELEASE_DONE_SQL,
            params![now.to_rfc3339(), owner, id.to_string()],
        )
        .map_err(map_sqlite_err)?;
    if affected == 0 {
        return Err(LeaseError::NotOwner.into());
    }
    Ok(ReleaseOutcome {
        status: "done".to_owned(),
        reason: None,
    })
}

/// Читает текущее число попыток, чтобы решить между «назад в очередь» и
/// «заблокировать» — двухшаговое (прочитать, затем записать), в отличие от
/// одностатейного [`CLAIM_SQL`]. Это безопасно именно потому, что итоговая
/// запись всё равно защищена условием `lease.owner = ?` в WHERE: если аренда
/// истекла и наряд перевыдан между чтением и записью, `affected == 0` и
/// вызывающий получает `NotOwner`, а не испорченную запись поверх нового
/// владельца. Атомарность одним предложением здесь не нужна — гонка,
/// которую решает [`CLAIM_SQL`], это гонка нескольких *взятий*; у отпускания
/// такой гонки нет, владелец ровно один.
fn release_failed(conn: &Connection, id: Uuid, owner: &str) -> anyhow::Result<ReleaseOutcome> {
    let id_str = id.to_string();
    let node = super::get_node(conn, &id_str)?.ok_or(LeaseError::NotOwner)?;
    let current_owner = node
        .data
        .get("lease")
        .and_then(|l| l.get("owner"))
        .and_then(|o| o.as_str());
    if current_owner != Some(owner) {
        return Err(LeaseError::NotOwner.into());
    }
    let attempts = node
        .data
        .get("attempts")
        .and_then(|a| a.as_i64())
        .unwrap_or(0);
    let now = Utc::now();

    if attempts >= MAX_ATTEMPTS {
        let reason = format!("исчерпаны попытки ({attempts} из {MAX_ATTEMPTS})");
        let affected = conn
            .execute(
                RELEASE_BLOCK_SQL,
                params![reason, now.to_rfc3339(), owner, id_str],
            )
            .map_err(map_sqlite_err)?;
        if affected == 0 {
            return Err(LeaseError::NotOwner.into());
        }
        return Ok(ReleaseOutcome {
            status: "blocked".to_owned(),
            reason: Some(reason),
        });
    }

    let cooldown_until = (now + chrono::Duration::minutes(DEFAULT_COOLDOWN_MINUTES)).to_rfc3339();
    let affected = conn
        .execute(
            RELEASE_RETRY_SQL,
            params![cooldown_until, now.to_rfc3339(), owner, id_str],
        )
        .map_err(map_sqlite_err)?;
    if affected == 0 {
        return Err(LeaseError::NotOwner.into());
    }
    Ok(ReleaseOutcome {
        status: "backlog".to_owned(),
        reason: None,
    })
}

/// Исполнитель распознал упор в человека. Блокирует задачу с причиной и
/// **не** возвращает её в очередь — сдача есть информация человеку, а не
/// повод для повтора (FR-014).
pub fn give_up(conn: &Connection, id: Uuid, owner: &str, why: &str) -> anyhow::Result<()> {
    if why.trim().is_empty() {
        anyhow::bail!("--why не может быть пустым");
    }
    let now = Utc::now();
    let affected = conn
        .execute(
            GIVE_UP_SQL,
            params![why, now.to_rfc3339(), owner, id.to_string()],
        )
        .map_err(map_sqlite_err)?;
    if affected == 0 {
        return Err(LeaseError::NotOwner.into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Разметка исполнимости (спека 006, фаза 3, data-model.md «Вердикт
// исполнимости»). Ступень 1 — детерминированные гейты, без модели: перенос
// одноразового скрипта `fitness-dryrun.mjs` (research.md, «Волна 0:
// результат») плюс два гейта, которых скрипт не содержал и которые волна 0
// вскрыла ручной сверкой уже ПОСЛЕ прогона — эпик (FR-005b) и запрещённое
// действие (FR-005c).
// ---------------------------------------------------------------------------

/// Вердикт исполнимости: тот же текст, что хранится в `data.fitness.verdict`
/// и что принимает `--verdict` у `au task fitness`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitnessVerdict {
    /// Машина закроет сама — единственный вердикт, допускающий наряд
    /// (FR-005).
    Machine,
    /// Нужен человек: либо нет проверяемого критерия, либо задача больше
    /// наряда, либо критерий требует запрещённого действия.
    Human,
    /// Часть критериев машинная, часть требует человека — откладывается
    /// целиком, без автоматического расщепления (FR-005a).
    Split,
}

impl FitnessVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            FitnessVerdict::Machine => "machine",
            FitnessVerdict::Human => "human",
            FitnessVerdict::Split => "split",
        }
    }
}

impl std::fmt::Display for FitnessVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Итог гейтов: вердикт вместе с обязательным обоснованием (FR-003a) — какой
/// критерий признан проверяемым действием, а при отказе — почему ни один не
/// признан.
#[derive(Debug, Clone)]
pub struct FitnessOutcome {
    pub verdict: FitnessVerdict,
    pub why: String,
}

/// Порог числа критериев приёмки, после которого задача считается эпиком, а
/// не одним нарядом (FR-005b). Взято по здравому смыслу, как и прочие
/// потолки прогона в research.md, — не измерено.
const EPIC_CRITERIA_THRESHOLD: usize = 6;

/// Скомпилированные регэкспы гейтов — строятся один раз на процесс, а не на
/// каждый вызов `evaluate_fitness` (прогон `--dry-run` вызывает его на всей
/// открытой очереди).
struct FitnessGates {
    /// Критерий-код в обратных кавычках — самый честный признак команды.
    inline_code: Regex,
    /// Критерий начинается с команды: «npm test зелёный», «cargo clippy чист».
    starts_with_cmd: Regex,
    /// Команда упомянута где-то в строке, с границей слова по обе стороны.
    cmd_at: Regex,
    /// Слово, говорящее об исходе запуска, а не о состоянии мира.
    green_marker: Regex,
    /// Формулировки проверки с однозначным исходом без имени команды.
    verifiable_shape: Vec<Regex>,
    /// Маркеры того, что задача упирается в другого человека.
    waits_for_human: Vec<Regex>,
    /// Результат живёт в голове, а не в файловой системе.
    unverifiable_intent: Vec<Regex>,
    /// `[project] ` — префикс лейбла, который снимается перед проверкой intent.
    label_prefix: Regex,
    /// Маркеры многоэтапности: эпик, фаза/этап/stage N, MVP-N.
    epic_marker: Vec<Regex>,
    /// Действия, запрещённые автономной работе (FR-025..FR-027).
    forbidden_action: Vec<Regex>,
}

/// Имена, при виде которых строка перестаёт быть намерением и становится
/// командой. Список — дословный перенос `RUNNABLE` из `fitness-dryrun.mjs`.
const RUNNABLE_COMMANDS: &[&str] = &[
    "cargo",
    "npm",
    "npx",
    "pnpm",
    "yarn",
    "node",
    "deno",
    "bun",
    "au",
    "aurelius",
    "git",
    "gh",
    "docker",
    "make",
    "bash",
    "sh",
    "pwsh",
    "powershell",
    "python",
    "python3",
    "pip",
    "pytest",
    "ruff",
    "mypy",
    "curl",
    "wget",
    "sqlite3",
    "psql",
    "redis-cli",
    "ssh",
    "systemctl",
    "cmake",
    "clippy",
    "rustfmt",
    "eslint",
    "tsc",
    "vitest",
    "jest",
    "playwright",
];

/// Компилирует регэксп из литерала, известного правильным на этапе
/// написания кода, — не из пользовательского ввода. Тот же приём, что в
/// `probes.rs`: `.expect` здесь не бьёт по принципу III, потому что упасть
/// он может только на опечатке в исходнике, а не на данных прогона.
fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("статический регэксп гейтов fitness")
}

fn build_gates() -> FitnessGates {
    let mut words: Vec<&str> = RUNNABLE_COMMANDS.to_vec();
    words.sort_by_key(|w| std::cmp::Reverse(w.len()));
    let cmd_alt = words
        .iter()
        .map(|w| regex::escape(w))
        .collect::<Vec<_>>()
        .join("|");

    FitnessGates {
        inline_code: re(r"`[^`]{2,}`"),
        starts_with_cmd: re(&format!(r"(?i)^\s*(?:команда\s+)?({cmd_alt})\b")),
        cmd_at: re(&format!(r#"(?i)(^|[`"'(\s])({cmd_alt})\b[\s:]"#)),
        green_marker: re(
            r"(?i)(зелён|зелен|проход(ит|ят)|успешн|без\s+ошибок|green|passes?|ok\b|чист|exit\s*0)",
        ),
        verifiable_shape: vec![
            re(r"(?i)exit\s*code\s*0"),
            re(r"(?i)код\s+возврата\s+(ноль|0)"),
            re(r"(?i)завершается\s+успешно"),
            re(r"(?i)проход(ит|ят)\s+(тест|проверк|сборк)"),
            re(r"(?i)тест(ы)?\s+(проход|зелён|зелен)"),
            re(r"(?i)сборка\s+(проход|зелён|зелен|успешн)"),
            re(r"(?i)собирается\s+без\s+ошибок"),
            re(r"(?i)без\s+ошибок\s+компил"),
            re(r"(?i)возвращает\s+(ноль|0|код)"),
            re(r"(?i)quick_check\s+(зелён|зелен|ok)"),
            re(r"(?i)integrity\s+ok"),
        ],
        waits_for_human: vec![
            re(r"(?i)дожд(аться|ись|ёмся)"),
            re(r"(?i)ответ(а|ы)?\s+(мерчант|клиент|партнёр|партнер|провайдер|поддержк)"),
            re(r"(?i)соглас(ие|ования|овать|уй)"),
            re(r"(?i)созвон"),
            re(r"(?i)связаться\s+с"),
            re(r"(?i)обсудить\s+с"),
            re(r"(?i)договорит[ьс]"),
            re(r"решить\s+с\s+\p{Lu}"),
            re(r"(?i)спросить\s+у"),
            re(r"(?i)подтвердить\s+у"),
            re(r"(?i)написать\s+(письмо|мерчант|клиент|провайдер|в\s+поддержк|тео|владельц)"),
            re(r"(?i)отправить\s+(претензи|письмо|счёт|счет|сообщени)"),
            re(r"(?i)выставить\s+счёт|выставить\s+счет"),
            re(r"(?i)оплатить|перевести\s+деньги"),
        ],
        unverifiable_intent: vec![
            re(r"(?i)^изучить"),
            re(r"(?i)^разобраться"),
            re(r"(?i)^описать"),
            re(r"(?i)^продумать"),
            re(r"(?i)^обдумать"),
            re(r"(?i)^оценить"),
            re(r"(?i)^прочитать"),
            re(r"(?i)^почитать"),
            re(r"(?i)^посмотреть"),
            re(r"(?i)^подумать"),
            re(r"(?i)^исследовать"),
            re(r"(?i)^понять"),
            re(r"(?i)^выяснить,?\s+как"),
            re(r"(?i)^определиться"),
        ],
        label_prefix: re(r"^\[[^\]]*\]\s*"),
        // FR-005b: перечисление фаз пишут и по-русски, и по-английски (спеки
        // этого репозитория сами используют «Phase N»/«US1») — оба варианта
        // равно достоверный сигнал многоэтапности, не только русский.
        epic_marker: vec![
            re(r"(?i)эпик"),
            re(r"(?i)фаза\s*\d"),
            re(r"(?i)phase\s*\d"),
            re(r"(?i)этап\s*\d"),
            re(r"(?i)stage\s*\d"),
            re(r"(?i)mvp[-\s]?\d"),
        ],
        // FR-005c: публикация, PR, слияние в основную линию, сообщение
        // человеку — исполнителю запрещено закрывать такой критерий честно
        // (FR-025..FR-027), сколько бы попыток он ни истратил.
        forbidden_action: vec![
            re(r"(?i)\bpr\b"),
            re(r"(?i)pull\s*request"),
            re(r"(?i)(слить|смерж\w*|мерж\w*)\s+(в|to|into)?\s*(main|master|основн\w*)"),
            re(r"(?i)запушить\s+в\s+(main|master)"),
            re(r"(?i)опубликова\w*"),
            re(r"(?i)публикаци\w*"),
            re(r"(?i)npm\s+publish"),
            re(r"(?i)cargo\s+publish"),
            re(r"(?i)отправ(ить|ь)\s+(сообщени|письмо|уведомлени)"),
        ],
    }
}

fn gates() -> &'static FitnessGates {
    static GATES: OnceLock<FitnessGates> = OnceLock::new();
    GATES.get_or_init(build_gates)
}

fn any_match(list: &[Regex], s: &str) -> bool {
    list.iter().any(|re| re.is_match(s))
}

/// `true`, если критерий приёмки — проверка с однозначным исходом, а не
/// намерение (FR-002). Три способа за это сказать, по убыванию честности:
/// команда в обратных кавычках, критерий начинается с команды, команда
/// упомянута рядом со словом об исходе; либо форма проверки без имени
/// команды вовсе («тесты проходят»).
fn looks_runnable(criterion: &str) -> bool {
    let g = gates();
    if g.inline_code.is_match(criterion) {
        return true;
    }
    if g.starts_with_cmd.is_match(criterion) {
        return true;
    }
    if g.cmd_at.is_match(criterion) && g.green_marker.is_match(criterion) {
        return true;
    }
    g.verifiable_shape.iter().any(|re| re.is_match(criterion))
}

fn truncate_for_why(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

/// Непустые критерии приёмки из `data.acceptance_criteria` — общая точка,
/// которой пользуются и гейты, и запись вердикта, и `au task fitness
/// --dry-run`, чтобы не заводить чтение JSON заново в каждом месте.
pub fn task_acceptance_criteria(data: &Value) -> Vec<String> {
    data.get("acceptance_criteria")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Сканирует лейбл и критерии одной строкой — **без** `note`. Эпик и
/// запрещённое действие ищутся в контракте задачи (заголовок, критерии), а
/// не в вольном тексте описания: `note` часто пересказывает историю («найдено
/// при аудите эпика 119», «PR #296 ждёт мержа») и слово «эпик»/«PR» там может
/// говорить о прошлом или о чужой задаче, а не о размере или закрытии этой.
/// Все три известных эпика волны 0 и оба известных запрета названы в лейбле
/// или в самом критерии — сужение до них не теряет ни одного подтверждённого
/// случая, только убирает ложные срабатывания на пересказе истории.
fn scoped_text(label: &str, criteria: &[String]) -> String {
    let mut s = label.to_owned();
    for c in criteria {
        s.push('\n');
        s.push_str(c);
    }
    s
}

/// FR-005b: объём задачи заведомо больше одного наряда. Считается эпиком по
/// маркеру многоэтапности в тексте, либо по числу критериев приёмки —
/// настоящий критерий проходит все прочие гейты, но закрыть эпик за один
/// наряд физически нельзя (research.md, «Волна 0: результат»).
fn epic_marker_hit(text: &str, criteria_count: usize) -> Option<String> {
    let g = gates();
    for re in &g.epic_marker {
        if let Some(m) = re.find(text) {
            return Some(format!("маркер многоэтапности «{}»", m.as_str()));
        }
    }
    if criteria_count > EPIC_CRITERIA_THRESHOLD {
        return Some(format!(
            "{criteria_count} критериев приёмки — больше одного наряда (порог {EPIC_CRITERIA_THRESHOLD})"
        ));
    }
    None
}

/// FR-005c: критерий требует действия, запрещённого автономной работе.
/// Задача машинная по форме, но закрыть её честно нельзя — исполнитель
/// упрётся в собственный предохранитель и трижды провалится на своём же
/// запрете (research.md, «Волна 0: результат», задача ledgent).
fn forbidden_action_hit(text: &str) -> Option<String> {
    let g = gates();
    for re in &g.forbidden_action {
        if let Some(m) = re.find(text) {
            return Some(format!("запрещённое действие «{}»", m.as_str()));
        }
    }
    None
}

/// Гейты отсева — ступень 1 разметки исполнимости, без модели (data-model.md,
/// «Вердикт исполнимости»). Детерминированный порядок проверок:
///
/// 1. нет критериев → `human` (FR-002);
/// 2. ни один критерий не проверяем → `human`, с уточнением «изучить /
///    описать» для намерений без результата (FR-002a);
/// 3. есть маркер ожидания человека — в теле или в критерии → `split`,
///    целиком, без расщепления (FR-005a);
/// 4. эпик или запрещённое действие → `human` (FR-005b, FR-005c);
/// 5. иначе → `machine`.
pub fn evaluate_fitness(label: &str, note: Option<&str>, criteria: &[String]) -> FitnessOutcome {
    let g = gates();

    if criteria.is_empty() {
        return FitnessOutcome {
            verdict: FitnessVerdict::Human,
            why: "нет ни одного критерия приёмки".to_owned(),
        };
    }

    let runnable: Vec<&String> = criteria.iter().filter(|c| looks_runnable(c)).collect();

    if runnable.is_empty() {
        let bare_label = g.label_prefix.replace(label, "");
        let intent_only = any_match(&g.unverifiable_intent, &bare_label);
        let why = if intent_only {
            "результат не проверяется отдельным действием (изучить / описать / разобраться); \
             отчёт исполнителя такой проверкой не считается"
                .to_owned()
        } else {
            format!(
                "ни один критерий не является запускаемой командой: {}",
                truncate_for_why(&criteria[0], 90)
            )
        };
        return FitnessOutcome {
            verdict: FitnessVerdict::Human,
            why,
        };
    }

    let body = format!("{label}\n{}", note.unwrap_or(""));
    let human_in_body = any_match(&g.waits_for_human, &body);
    let human_in_criteria = criteria.iter().any(|c| any_match(&g.waits_for_human, c));

    if human_in_body || human_in_criteria {
        return FitnessOutcome {
            verdict: FitnessVerdict::Split,
            why: format!(
                "есть проверяемый критерий ({}), но задача упирается в человека — откладывается целиком",
                truncate_for_why(runnable[0], 70)
            ),
        };
    }

    let text = scoped_text(label, criteria);
    if let Some(marker) = epic_marker_hit(&text, criteria.len()) {
        return FitnessOutcome {
            verdict: FitnessVerdict::Human,
            why: format!("объём задачи больше одного наряда: {marker}"),
        };
    }
    if let Some(hit) = forbidden_action_hit(&text) {
        return FitnessOutcome {
            verdict: FitnessVerdict::Human,
            why: format!("критерий запрещён автономной работе: {hit}"),
        };
    }

    FitnessOutcome {
        verdict: FitnessVerdict::Machine,
        why: format!(
            "проверяемый критерий: {}",
            truncate_for_why(runnable[0], 110)
        ),
    }
}

/// Хеш содержания задачи для `fitness.of_hash` (FR-004). Отдельно от
/// `content_hash` в столбце `nodes` — тот про дедупликацию сессий и у
/// существующих задач почти всегда `NULL`; этот про то, изменился ли текст,
/// от которого зависит вердикт.
fn fitness_content_hash(label: &str, note: Option<&str>, criteria: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(label.as_bytes());
    hasher.update(b"\x1f");
    hasher.update(note.unwrap_or("").as_bytes());
    for c in criteria {
        hasher.update(b"\x1e");
        hasher.update(c.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// `true`, если сохранённый `of_hash` совпадает с хешем текущего содержания
/// — вердикт ещё не протух. Несовпадение делает его недействительным
/// независимо от того, что записано в `verdict` (FR-004).
pub fn fitness_is_current(
    label: &str,
    note: Option<&str>,
    criteria: &[String],
    of_hash: &str,
) -> bool {
    fitness_content_hash(label, note, criteria) == of_hash
}

const SET_FITNESS_SQL: &str = "
UPDATE nodes SET
  data = json_set(data, '$.fitness', json_object('verdict', ?1, 'why', ?2, 'of_hash', ?3)),
  updated_at = ?4, updated_by = ?5
WHERE id = ?6 AND deleted_at IS NULL AND node_type = '\"task\"'
";

/// Ставит вердикт исполнимости (FR-003, FR-003a, FR-004). Пишет ровно один
/// путь — `data.fitness` через `json_set`, а не общий `update_node`, которому
/// можно передать любой JSON: на уровне кода разметка физически не может
/// тронуть ничего другого в задаче (FR-003, T025).
pub fn set_fitness(
    conn: &Connection,
    id: Uuid,
    verdict: FitnessVerdict,
    why: &str,
) -> anyhow::Result<()> {
    let why = why.trim();
    if why.is_empty() {
        anyhow::bail!(
            "--why не может быть пустым — вердикт без обоснования считается отсутствующим (FR-003a)"
        );
    }
    let id_str = id.to_string();
    let node = super::get_node(conn, &id_str)?
        .ok_or_else(|| anyhow::anyhow!("задача не найдена: {id}"))?;
    let criteria = task_acceptance_criteria(&node.data);
    let hash = fitness_content_hash(&node.label, node.note.as_deref(), &criteria);
    let now = Utc::now();
    let author = crate::identity::current().map(|i| i.as_author());

    let affected = conn
        .execute(
            SET_FITNESS_SQL,
            params![
                verdict.as_str(),
                why,
                hash,
                now.to_rfc3339(),
                author,
                id_str
            ],
        )
        .map_err(map_sqlite_err)?;
    if affected == 0 {
        anyhow::bail!("задача не найдена или удалена: {id}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::models::{MemoryKind, NodeType};
    use std::collections::HashSet;

    /// Тот же приём, что в `crud.rs`: настоящий temp-файл, не `:memory:` —
    /// `db::open` жёстко требует WAL, которого `:memory:` не умеет.
    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn new(tag: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "aurelius-lease-test-{tag}-{}.db",
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

    fn setup() -> (TmpDb, Connection) {
        let tmp = TmpDb::new("setup");
        let conn = db::open(&tmp.0).expect("open temp db");
        (tmp, conn)
    }

    /// Заводит задачу с вердиктом `machine`, готовую попасть в пул. Подкоманда
    /// `au task fitness`, которая в проекте ставит этот вердикт, — задача
    /// другой фазы (Phase 3), поэтому тесты наряда заводят его напрямую.
    fn seed_machine_task(conn: &Connection, extra: serde_json::Value) -> Uuid {
        let mut data = serde_json::json!({
            "status": "backlog",
            "priority": "medium",
            "attempts": 0,
            "fitness": {"verdict": "machine"},
        });
        let data_obj = data.as_object_mut().expect("object");
        if let Some(extra_obj) = extra.as_object() {
            for (k, v) in extra_obj {
                data_obj.insert(k.clone(), v.clone());
            }
        }
        crate::graph::add_node_full(
            conn,
            NodeType::Task,
            "тестовый наряд",
            None,
            "test",
            data,
            MemoryKind::Semantic,
            None,
        )
        .expect("insert machine task")
        .id
    }

    fn backdate(conn: &Connection, id: Uuid, path: &str, when: DateTime<Utc>) {
        conn.execute(
            &format!("UPDATE nodes SET data = json_set(data, '{path}', ?1) WHERE id = ?2"),
            params![when.to_rfc3339(), id.to_string()],
        )
        .expect("backdate field");
    }

    /// SC-005: два писателя гоняют взятие по 200 раз каждый на общей
    /// временной копии базы — каждый из 400 нарядов выдаётся ровно одному
    /// вызову, ни разу дважды. `rusqlite::Connection` не `Sync`, поэтому
    /// «два процесса» здесь — два потока с собственными подключениями к
    /// одному файлу, что и воспроизводит настоящий сценарий (много писателей,
    /// один локальный файл — принцип II конституции).
    #[test]
    fn concurrent_claims_never_double_issue() {
        let tmp = TmpDb::new("concurrent");
        const PER_THREAD: usize = 200;
        {
            let conn = db::open(&tmp.0).expect("open temp db (seed)");
            for _ in 0..PER_THREAD * 2 {
                seed_machine_task(&conn, serde_json::json!({}));
            }
        }

        let claim_all = |path: std::path::PathBuf, tag: &'static str| {
            std::thread::spawn(move || {
                let conn = db::open(&path).expect("open temp db (thread)");
                let mut ids = Vec::with_capacity(PER_THREAD);
                for i in 0..PER_THREAD {
                    let owner = format!("{tag}/{i}");
                    let task = claim(&conn, &owner, tag, 60, None)
                        .unwrap_or_else(|e| panic!("claim #{i} on {tag} failed: {e:#}"));
                    ids.push(task.id);
                }
                ids
            })
        };

        let handle_a = claim_all(tmp.0.clone(), "thread-a");
        let handle_b = claim_all(tmp.0.clone(), "thread-b");
        let mut ids = handle_a.join().expect("thread-a panicked");
        ids.extend(handle_b.join().expect("thread-b panicked"));

        assert_eq!(
            ids.len(),
            PER_THREAD * 2,
            "каждый вызов обязан выдать наряд"
        );
        let unique: HashSet<_> = ids.iter().collect();
        assert_eq!(
            unique.len(),
            ids.len(),
            "двойное взятие: какой-то id выдан больше одного раза"
        );
    }

    /// SC-006, FR-007: до истечения аренды задача недоступна; после — снова в
    /// пуле, и `attempts` уже увеличен вторым взятием.
    #[test]
    fn abandoned_lease_returns_to_pool_after_expiry_with_attempts_bumped() {
        let (_tmp, conn) = setup();
        let id = seed_machine_task(&conn, serde_json::json!({}));

        let first = claim(&conn, "owner-a", "run-1", 60, None).expect("first claim");
        assert_eq!(first.id, id);
        assert_eq!(first.data["attempts"], 1);

        let err = claim(&conn, "owner-b", "run-2", 60, None).unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<LeaseError>(),
                Some(LeaseError::NoTasksAvailable)
            ),
            "аренда ещё живая — наряд не должен быть доступен: {err:#}"
        );

        // Тест обгоняет часы, а не ждёт настоящий час аренды.
        backdate(
            &conn,
            id,
            "$.lease.until",
            Utc::now() - chrono::Duration::minutes(1),
        );

        let second = claim(&conn, "owner-b", "run-2", 60, None).expect("reclaim after expiry");
        assert_eq!(second.id, id);
        assert_eq!(
            second.data["attempts"], 2,
            "повторное взятие обязано увеличить attempts"
        );
    }

    /// FR-008, SC-008: после третьей неудачи — `blocked` с причиной, в пул не
    /// попадает даже если остывание снято.
    #[test]
    fn third_failure_blocks_task_and_removes_it_from_pool() {
        let (_tmp, conn) = setup();
        let id = seed_machine_task(&conn, serde_json::json!({}));

        for attempt in 1..=3 {
            let owner = format!("owner-{attempt}");
            let claimed = claim(&conn, &owner, "run", 60, None)
                .unwrap_or_else(|e| panic!("claim #{attempt} failed: {e:#}"));
            assert_eq!(claimed.id, id);

            let outcome = release(&conn, id, &owner, Verdict::Failed, "cargo test упал")
                .expect("release failed");

            if attempt < 3 {
                assert_eq!(outcome.status, "backlog");
                // Реальное остывание — минуты; тест обгоняет часы, а не ждёт их.
                backdate(
                    &conn,
                    id,
                    "$.cooldown_until",
                    Utc::now() - chrono::Duration::minutes(1),
                );
            } else {
                assert_eq!(outcome.status, "blocked");
                let reason = outcome.reason.expect("причина обязана быть записана");
                assert!(
                    reason.contains('3'),
                    "причина обязана называть число попыток: {reason}"
                );
            }
        }

        // Даже без остывания заблокированная задача не должна снова браться.
        backdate(
            &conn,
            id,
            "$.cooldown_until",
            Utc::now() - chrono::Duration::minutes(1),
        );
        let err = claim(&conn, "owner-4", "run", 60, None).unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<LeaseError>(),
                Some(LeaseError::NoTasksAvailable)
            ),
            "заблокированная задача не обязана выдаваться: {err:#}"
        );

        let node = crate::graph::get_node(&conn, &id.to_string())
            .expect("query node")
            .expect("node exists");
        assert_eq!(node.data["status"], "blocked");
    }

    /// FR-009: продление — не молчаливый успех для чужого наряда.
    #[test]
    fn renew_refuses_when_not_owner() {
        let (_tmp, conn) = setup();
        let id = seed_machine_task(&conn, serde_json::json!({}));
        claim(&conn, "owner-a", "run-1", 60, None).expect("claim");

        let ok = renew(&conn, id, "owner-a", 60);
        assert!(ok.is_ok(), "владелец обязан суметь продлить: {ok:?}");

        let refused = renew(&conn, id, "owner-b", 60).unwrap_err();
        assert!(matches!(
            refused.downcast_ref::<LeaseError>(),
            Some(LeaseError::NotOwner)
        ));
    }

    /// FR-014: сдача блокирует и не возвращает в очередь — в отличие от
    /// `release --verdict failed`, которое возвращает.
    #[test]
    fn give_up_blocks_without_returning_to_pool() {
        let (_tmp, conn) = setup();
        let id = seed_machine_task(&conn, serde_json::json!({}));
        claim(&conn, "owner-a", "run-1", 60, None).expect("claim");

        give_up(&conn, id, "owner-a", "нужен человек: согласие клиента").expect("give up");

        let node = crate::graph::get_node(&conn, &id.to_string())
            .expect("query node")
            .expect("node exists");
        assert_eq!(node.data["status"], "blocked");
        assert_eq!(node.data["blocked_by"], "нужен человек: согласие клиента");

        let err = claim(&conn, "owner-b", "run-2", 60, None).unwrap_err();
        assert!(matches!(
            err.downcast_ref::<LeaseError>(),
            Some(LeaseError::NoTasksAvailable)
        ));
    }

    /// FR-015: закрытие ставит `done_by=smena`, чтобы ложные закрытия ночи
    /// откатывались одним запросом.
    #[test]
    fn release_done_marks_done_by_smena() {
        let (_tmp, conn) = setup();
        let id = seed_machine_task(&conn, serde_json::json!({}));
        claim(&conn, "owner-a", "run-1", 60, None).expect("claim");

        let outcome = release(&conn, id, "owner-a", Verdict::Done, "cargo test зелёный")
            .expect("release done");
        assert_eq!(outcome.status, "done");

        let node = crate::graph::get_node(&conn, &id.to_string())
            .expect("query node")
            .expect("node exists");
        assert_eq!(node.data["status"], "done");
        assert_eq!(node.data["done_by"], "smena");
    }

    // --- T026: разметка исполнимости (фаза 3) ------------------------------

    /// Заводит задачу с произвольными критериями приёмки, без вердикта —
    /// разметка тестов сама решает, каким его ставить.
    fn seed_task_with(
        conn: &Connection,
        label: &str,
        note: Option<&str>,
        criteria: &[&str],
        extra: serde_json::Value,
    ) -> Uuid {
        let mut data = serde_json::json!({
            "status": "backlog",
            "priority": "medium",
            "acceptance_criteria": criteria,
        });
        let data_obj = data.as_object_mut().expect("object");
        if let Some(extra_obj) = extra.as_object() {
            for (k, v) in extra_obj {
                data_obj.insert(k.clone(), v.clone());
            }
        }
        crate::graph::add_node_full(
            conn,
            NodeType::Task,
            label,
            note,
            "test",
            data,
            MemoryKind::Semantic,
            None,
        )
        .expect("insert task")
        .id
    }

    /// Критерий-команда без маркеров ожидания человека — `machine`.
    #[test]
    fn command_criterion_gives_machine() {
        let outcome = evaluate_fitness(
            "[boostix] чинит парсер",
            None,
            &["cargo test --workspace зелёный".to_owned()],
        );
        assert_eq!(outcome.verdict, FitnessVerdict::Machine);
        assert!(!outcome.why.is_empty(), "обоснование обязано быть непустым");
        assert!(outcome.why.contains("cargo"));
    }

    /// Единственный критерий — маркер ожидания человека, команды нет — `human`.
    #[test]
    fn human_wait_marker_gives_human() {
        let outcome = evaluate_fitness(
            "[boostix] получить креды",
            None,
            &["дождаться ответа мерчанта".to_owned()],
        );
        assert_eq!(outcome.verdict, FitnessVerdict::Human);
        assert!(!outcome.why.is_empty());
    }

    /// Ни одного критерия приёмки — `human`.
    #[test]
    fn no_criteria_gives_human() {
        let outcome = evaluate_fitness("[boostix] сделать что-то", None, &[]);
        assert_eq!(outcome.verdict, FitnessVerdict::Human);
        assert_eq!(outcome.why, "нет ни одного критерия приёмки");
    }

    /// Смешанные критерии — часть машинная, часть про человека — `split`, и
    /// после записи такая задача не должна попадать в машинный пул (FR-005a).
    #[test]
    fn mixed_criteria_give_split_and_stay_out_of_pool() {
        let criteria = [
            "cargo test --workspace зелёный",
            "дождаться ответа провайдера",
        ];
        let owned: Vec<String> = criteria.iter().map(|s| (*s).to_owned()).collect();
        let outcome = evaluate_fitness("[boostix] получить боевые креды", None, &owned);
        assert_eq!(outcome.verdict, FitnessVerdict::Split);

        let (_tmp, conn) = setup();
        let id = seed_task_with(
            &conn,
            "[boostix] получить боевые креды",
            None,
            &criteria,
            serde_json::json!({}),
        );
        set_fitness(&conn, id, FitnessVerdict::Split, &outcome.why).expect("set fitness");

        let err = claim(&conn, "owner-a", "run-1", 60, None).unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<LeaseError>(),
                Some(LeaseError::NoTasksAvailable)
            ),
            "split-вердикт не должен допускать задачу в пул: {err:#}"
        );
    }

    /// Настоящий критерий, но объём — на дни работы (маркер многоэтапности в
    /// лейбле) — `human`, не `machine` (FR-005b).
    #[test]
    fn epic_task_gives_human() {
        let outcome = evaluate_fitness(
            "[blyss-core] Реализовать Phase 3 (US1): живой голосовой круг",
            None,
            &["cargo test --workspace зелёный".to_owned()],
        );
        assert_eq!(outcome.verdict, FitnessVerdict::Human);
        assert!(
            outcome.why.contains("наряда"),
            "обоснование обязано называть причину отказа — объём: {}",
            outcome.why
        );
    }

    /// Настоящий критерий, но его закрытие требует запрещённого действия
    /// (PR) — `human`, не `machine` (FR-005c).
    #[test]
    fn forbidden_pr_criterion_gives_human() {
        let outcome = evaluate_fitness(
            "[ledgent] Commit and PR the report feature",
            None,
            &["npm run verify зелёный в CI".to_owned()],
        );
        assert_eq!(outcome.verdict, FitnessVerdict::Human);
        assert!(
            outcome.why.contains("запрещ"),
            "обоснование обязано называть запрет: {}",
            outcome.why
        );
    }

    /// FR-004: изменение текста задачи делает записанный вердикт
    /// недействительным — сверяется хеш содержания, а не факт наличия записи.
    #[test]
    fn changed_content_invalidates_verdict() {
        let (_tmp, conn) = setup();
        let id = seed_task_with(
            &conn,
            "[boostix] чинит парсер",
            None,
            &["cargo test --workspace зелёный"],
            serde_json::json!({}),
        );
        set_fitness(
            &conn,
            id,
            FitnessVerdict::Machine,
            "проверяемый критерий: cargo test",
        )
        .expect("set fitness");

        let node = crate::graph::get_node(&conn, &id.to_string())
            .expect("query node")
            .expect("node exists");
        let of_hash = node.data["fitness"]["of_hash"]
            .as_str()
            .expect("hash written")
            .to_owned();
        let criteria = task_acceptance_criteria(&node.data);

        assert!(
            fitness_is_current(&node.label, node.note.as_deref(), &criteria, &of_hash),
            "хеш обязан совпасть сразу после записи"
        );
        assert!(
            !fitness_is_current(
                "[boostix] чинит другой парсер",
                node.note.as_deref(),
                &criteria,
                &of_hash
            ),
            "изменённый текст задачи обязан протухнуть вердикт"
        );
    }

    /// FR-003, T025: разметка не имеет права трогать ничего, кроме `fitness`
    /// — на уровне кода, а не соглашения.
    #[test]
    fn set_fitness_touches_only_fitness_field() {
        let (_tmp, conn) = setup();
        let id = seed_task_with(
            &conn,
            "[boostix] чинит парсер",
            None,
            &["cargo test --workspace зелёный"],
            serde_json::json!({"priority": "high", "custom_marker": "не трогать"}),
        );

        set_fitness(
            &conn,
            id,
            FitnessVerdict::Machine,
            "проверяемый критерий: cargo test",
        )
        .expect("set fitness");

        let node = crate::graph::get_node(&conn, &id.to_string())
            .expect("query node")
            .expect("node exists");
        assert_eq!(node.data["status"], "backlog");
        assert_eq!(node.data["priority"], "high");
        assert_eq!(node.data["custom_marker"], "не трогать");
        assert_eq!(node.data["fitness"]["verdict"], "machine");
    }

    /// FR-003a: пустое обоснование — вердикт считается отсутствующим, запись
    /// отклоняется.
    #[test]
    fn set_fitness_rejects_empty_why() {
        let (_tmp, conn) = setup();
        let id = seed_task_with(
            &conn,
            "[boostix] чинит парсер",
            None,
            &["cargo test --workspace зелёный"],
            serde_json::json!({}),
        );
        let err = set_fitness(&conn, id, FitnessVerdict::Machine, "   ").unwrap_err();
        assert!(err.to_string().contains("пустым"));
    }
}
