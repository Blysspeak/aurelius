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

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, ErrorCode};
use serde_json::Value;
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
}
