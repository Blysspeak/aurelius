//! Живые напоминания: явно поставленный момент времени, а не производная от
//! чего-то ещё («созрела задача» — про читаемость улики, «наступил срок» —
//! про часы; `tasks::is_ripe` и это модуль нарочно не пересекаются).
//!
//! Владелец 2026-09-10 сформулировал три требования, и каждое из них — не
//! пожелание, а решение, зашитое в схему:
//!
//! 1. У напоминания явное состояние, и `done`/`cancelled` — два РАЗНЫХ
//!    исхода, а не один nullable timestamp: слить их значило бы стереть
//!    разницу между «сделано» и «не будет сделано» из истории.
//! 2. Перенос оставляет след: сколько раз напоминание сдвигали и откуда —
//!    честная мера того, что задачу откладывают, и это должно читаться
//!    постфактум ([`events`]), а не только угадываться по текущему `due_at`.
//! 3. У напоминания есть адресат: часть напоминаний — для человека, часть —
//!    для ИИ-сессии, большинство — для обоих, и именно адресат решает, каким
//!    каналом можно доставить ([`Owner`]).
//!
//! Правило, на котором держится вся конструкция: часами владеет РОВНО ОДИН
//! процесс — демон второй волны (см. `aurelius:reminders:clock-owner`).
//! Всякий другой читатель — потребитель, а не часовщик, и обязан доказывать,
//! что забрал напоминание, ПОБЕДОЙ В УСЛОВНОМ UPDATE ([`mark_delivered`]), а
//! не чтением состояния с последующей отдельной записью: гонку из двух
//! потребителей решает СУБД через `WHERE state = ...`, а не порядок вызовов
//! в коде.

use anyhow::Result;
use chrono::{DateTime, Duration, Local, NaiveDateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// Адресат напоминания — тот, кому можно его показать, и тем самым канал,
/// которому разрешено это сделать: `Me` — только внесессионный канал (десктоп,
/// телефон), `Ai` — только потребитель внутри сессии, `Both` — любой из них.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    Me,
    Ai,
    Both,
}

impl Owner {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Owner::Me => "me",
            Owner::Ai => "ai",
            Owner::Both => "both",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "me" => Some(Owner::Me),
            "ai" => Some(Owner::Ai),
            "both" => Some(Owner::Both),
            _ => None,
        }
    }
}

/// Состояние напоминания. `Pending` вооружено и ждёт своего момента.
/// `Delivered` показано кому-то и ждёт, чтобы человек или агент его
/// РАЗРЕШИЛИ — это НЕ исход, а промежуточная остановка. `Done` и `Cancelled`
/// — два разных исхода, и не путать их друг с другом — весь смысл того,
/// зачем это перечисление вообще заведено, а не булев флаг `resolved`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Pending,
    Delivered,
    Done,
    Cancelled,
}

impl State {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            State::Pending => "pending",
            State::Delivered => "delivered",
            State::Done => "done",
            State::Cancelled => "cancelled",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(State::Pending),
            "delivered" => Some(State::Delivered),
            "done" => Some(State::Done),
            "cancelled" => Some(State::Cancelled),
            _ => None,
        }
    }
}

/// Один шаг журнала одного напоминания ([`ReminderEvent`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Created,
    Delivered,
    Snoozed,
    Rearmed,
    Done,
    Cancelled,
}

impl EventKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Created => "created",
            EventKind::Delivered => "delivered",
            EventKind::Snoozed => "snoozed",
            EventKind::Rearmed => "rearmed",
            EventKind::Done => "done",
            EventKind::Cancelled => "cancelled",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "created" => Some(EventKind::Created),
            "delivered" => Some(EventKind::Delivered),
            "snoozed" => Some(EventKind::Snoozed),
            "rearmed" => Some(EventKind::Rearmed),
            "done" => Some(EventKind::Done),
            "cancelled" => Some(EventKind::Cancelled),
            _ => None,
        }
    }
}

/// Зеркало строки таблицы `reminders` (миграция V15, `db.rs`).
#[derive(Debug, Clone, Serialize)]
pub struct Reminder {
    pub id: String,
    pub task_id: Option<String>,
    pub project: Option<String>,
    pub text: String,
    pub owner: Owner,
    pub state: State,
    pub due_at: DateTime<Utc>,
    /// Момент, на который напоминание было поставлено ПЕРВЫЙ раз — не
    /// трогается ни `snooze`, ни повторным взводом (`mark_delivered`).
    pub original_due_at: DateTime<Utc>,
    pub repeat_spec: Option<String>,
    pub created_at: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub delivered_via: Option<String>,
    pub delivered_count: i64,
    /// Сколько раз напоминание перенесли — дешёвое чтение того же факта,
    /// который полностью хранит [`events`].
    pub snooze_count: i64,
    pub done_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
}

/// Одна запись журнала одного напоминания — то, что делает перенос видимым
/// постфактум, а не только предполагаемым по текущему `due_at`.
#[derive(Debug, Clone, Serialize)]
pub struct ReminderEvent {
    pub at: DateTime<Utc>,
    pub kind: EventKind,
    pub detail: Option<String>,
}

/// Данные для [`add`]. `due_at` — уже разобранный момент: разбором строки
/// (`--at`/`--in`) занимается вызывающий через [`parse_moment`]/[`parse_delay`],
/// этот модуль про хранение и состояние, а не про грамматику ввода.
pub struct NewReminder {
    pub text: String,
    pub due_at: DateTime<Utc>,
    pub owner: Owner,
    pub task_id: Option<String>,
    pub project: Option<String>,
    pub repeat_spec: Option<String>,
}

/// Список колонок в порядке полей [`Reminder`] — одна строка, а не
/// продублированная россыпь `SELECT *` по десяти функциям.
const REMINDER_COLUMNS: &str = "id, task_id, project, text, owner, state, due_at, \
     original_due_at, repeat_spec, created_at, delivered_at, delivered_via, \
     delivered_count, snooze_count, done_at, cancelled_at";

/// Сколько раз [`next_occurrence`] готова провернуть колесо повтора за один
/// вызов. Демон, простоявший три дня с повтором раз в минуту, сделает не
/// более этого числа шагов, прежде чем сдаться на «сейчас плюс период» — цикл
/// без потолка на испорченных данных (`repeat_spec`, разобранный в отрицательный
/// или нулевой интервал — хотя `parse_delay` их и не отдаёт) стал бы вечным.
const MAX_REARM_STEPS: u32 = 100_000;

/// unix-секунды из БД → `DateTime<Utc>`. `unwrap_or_default` — не потеря
/// данных, а защита от переполнения `i64`, которого при разумных датах не
/// бывает: `DateTime::<Utc>::default()` — это эпоха, а не паника.
fn from_unix(secs: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(secs, 0).unwrap_or_default()
}

fn row_to_reminder(row: &rusqlite::Row) -> rusqlite::Result<Reminder> {
    let owner_s: String = row.get(4)?;
    let state_s: String = row.get(5)?;
    Ok(Reminder {
        id: row.get(0)?,
        task_id: row.get(1)?,
        project: row.get(2)?,
        text: row.get(3)?,
        owner: Owner::parse(&owner_s).unwrap_or(Owner::Both),
        state: State::parse(&state_s).unwrap_or(State::Pending),
        due_at: from_unix(row.get(6)?),
        original_due_at: from_unix(row.get(7)?),
        repeat_spec: row.get(8)?,
        created_at: from_unix(row.get(9)?),
        delivered_at: row.get::<_, Option<i64>>(10)?.map(from_unix),
        delivered_via: row.get(11)?,
        delivered_count: row.get(12)?,
        snooze_count: row.get(13)?,
        done_at: row.get::<_, Option<i64>>(14)?.map(from_unix),
        cancelled_at: row.get::<_, Option<i64>>(15)?.map(from_unix),
    })
}

fn write_event(
    conn: &Connection,
    reminder_id: &str,
    at: DateTime<Utc>,
    kind: EventKind,
    detail: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO reminder_events (reminder_id, at, kind, detail) VALUES (?1, ?2, ?3, ?4)",
        params![reminder_id, at.timestamp(), kind.as_str(), detail],
    )?;
    Ok(())
}

fn get(conn: &Connection, id: &str) -> Result<Option<Reminder>> {
    Ok(conn
        .query_row(
            &format!("SELECT {REMINDER_COLUMNS} FROM reminders WHERE id = ?1"),
            [id],
            row_to_reminder,
        )
        .optional()?)
}

/// Число без знака, суффикс `m`/`h`/`d`/`w`, либо голое число — тогда это
/// минуты. Единственный разборщик задержки во всей команде: `--in`,
/// `--repeat`, `--remind-before` и снуз идут через него, поэтому у них нет
/// шанса разойтись в том, что значит «2h».
#[must_use]
pub fn parse_delay(spec: &str) -> Option<Duration> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    let (digits, unit) = match spec.chars().next_back() {
        Some(c) if c.is_ascii_alphabetic() => (&spec[..spec.len() - c.len_utf8()], c),
        _ => (spec, 'm'),
    };
    let n: i64 = digits.parse().ok()?;
    if n <= 0 {
        return None;
    }
    match unit {
        'm' => Some(Duration::minutes(n)),
        'h' => Some(Duration::hours(n)),
        'd' => Some(Duration::days(n)),
        'w' => Some(Duration::weeks(n)),
        _ => None,
    }
}

/// Момент из строки: RFC 3339, `ГГГГ-ММ-ДД ЧЧ:ММ` и `ГГГГ-ММ-ДД` в машинном
/// локальном времени, либо `ЧЧ:ММ` — ближайшее наступление этого времени
/// (сегодня, если оно ещё впереди, иначе завтра). Всё остальное — `None`, а
/// не паника: `TimeZone::from_local_datetime` возвращает `LocalResult`
/// вместо значения именно затем, чтобы неоднозначный момент (перевод часов)
/// не подставлялся наугад.
#[must_use]
pub fn parse_moment(spec: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let spec = spec.trim();

    if let Ok(dt) = DateTime::parse_from_rfc3339(spec) {
        return Some(dt.with_timezone(&Utc));
    }

    if let Ok(naive) = NaiveDateTime::parse_from_str(spec, "%Y-%m-%d %H:%M") {
        return Local
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.with_timezone(&Utc));
    }

    if let Ok(date) = chrono::NaiveDate::parse_from_str(spec, "%Y-%m-%d") {
        let naive = date.and_hms_opt(9, 0, 0)?;
        return Local
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.with_timezone(&Utc));
    }

    if let Ok(time) = chrono::NaiveTime::parse_from_str(spec, "%H:%M") {
        let local_now = now.with_timezone(&Local);
        let today = local_now.date_naive();
        let candidate = Local.from_local_datetime(&today.and_time(time)).single()?;
        let candidate = if candidate > local_now {
            candidate
        } else {
            let tomorrow = today.succ_opt()?;
            Local
                .from_local_datetime(&tomorrow.and_time(time))
                .single()?
        };
        return Some(candidate.with_timezone(&Utc));
    }

    None
}

/// Завести напоминание: `Pending`, `original_due_at == due_at`, запись
/// `Created` в журнале. `repeat_spec`, который [`parse_delay`] не разбирает,
/// — ошибка ЗДЕСЬ, а не строка, которая никогда не продвинется дальше первой
/// доставки.
pub fn add(conn: &Connection, new: NewReminder) -> Result<Reminder> {
    if let Some(spec) = &new.repeat_spec {
        if parse_delay(spec).is_none() {
            anyhow::bail!("--repeat: не разобрать задержку '{spec}'");
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    conn.execute(
        "INSERT INTO reminders
            (id, task_id, project, text, owner, state, due_at, original_due_at,
             repeat_spec, created_at, delivered_count, snooze_count)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?6, ?7, ?8, 0, 0)",
        params![
            id,
            new.task_id,
            new.project,
            new.text,
            new.owner.as_str(),
            new.due_at.timestamp(),
            new.repeat_spec,
            now.timestamp(),
        ],
    )?;
    write_event(conn, &id, now, EventKind::Created, None)?;
    get(conn, &id)?.ok_or_else(|| anyhow::anyhow!("напоминание пропало сразу после вставки: {id}"))
}

fn select_pending(
    conn: &Connection,
    threshold: DateTime<Utc>,
    owner_filter: Option<Owner>,
    limit: usize,
) -> Result<Vec<Reminder>> {
    let limit_i = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = match owner_filter {
        Some(owner) => {
            let mut stmt = conn.prepare(&format!(
                "SELECT {REMINDER_COLUMNS} FROM reminders
                  WHERE state = 'pending' AND due_at <= ?1 AND (owner = ?2 OR owner = 'both')
                  ORDER BY due_at ASC LIMIT ?3"
            ))?;
            let found: Vec<Reminder> = stmt
                .query_map(
                    params![threshold.timestamp(), owner.as_str(), limit_i],
                    row_to_reminder,
                )?
                .filter_map(std::result::Result::ok)
                .collect();
            found
        }
        None => {
            let mut stmt = conn.prepare(&format!(
                "SELECT {REMINDER_COLUMNS} FROM reminders
                  WHERE state = 'pending' AND due_at <= ?1
                  ORDER BY due_at ASC LIMIT ?2"
            ))?;
            let found: Vec<Reminder> = stmt
                .query_map(params![threshold.timestamp(), limit_i], row_to_reminder)?
                .filter_map(std::result::Result::ok)
                .collect();
            found
        }
    };
    Ok(rows)
}

/// Созревшие напоминания: `Pending` и `due_at <= now`, старейшие первыми.
/// Потребитель внутри сессии зовёт это с `Some(Owner::Ai)`.
pub fn due(
    conn: &Connection,
    now: DateTime<Utc>,
    owner_filter: Option<Owner>,
    limit: usize,
) -> Result<Vec<Reminder>> {
    select_pending(conn, now, owner_filter, limit)
}

/// То же самое, но `due_at <= now - grace`. Это то, чем демон второй волны
/// зовёт с `Some(Owner::Me)`: окно `grace` И ЕСТЬ защита от двойной
/// доставки, потому что живая сессия, уже забравшая напоминание
/// (`mark_delivered`), увела его из `Pending` — демону там просто нечего
/// найти перезревшим.
pub fn overdue_undelivered(
    conn: &Connection,
    now: DateTime<Utc>,
    grace: Duration,
    owner_filter: Option<Owner>,
    limit: usize,
) -> Result<Vec<Reminder>> {
    select_pending(conn, now - grace, owner_filter, limit)
}

/// Момент строго позже `now`, до которого доводит `from`, шагая `delay` за
/// раз. Демон, простоявший три дня при повторе раз в час, доберёт ровно один
/// будущий момент, а не выстрелит трижды подряд по пропущенным часам.
/// Потолок шагов — [`MAX_REARM_STEPS`]; за ним — `now + delay`, а не
/// зависший цикл.
fn next_occurrence(from: DateTime<Utc>, delay: Duration, now: DateTime<Utc>) -> DateTime<Utc> {
    let mut next = from;
    for _ in 0..MAX_REARM_STEPS {
        if next > now {
            return next;
        }
        next += delay;
    }
    now + delay
}

/// Условный `UPDATE`, который и есть единственный судья: побеждает ровно
/// один из гоняющихся вызовов, потому что проверка состояния стоит в
/// `WHERE`, а не в отдельном чтении до него. Возвращает, взял ли ИМЕННО этот
/// вызов напоминание; `Delivered`-событие с `via` в деталях пишется только
/// тогда. Повторяющееся напоминание не остаётся в `Delivered` — оно тут же
/// перевзводится в `Pending` со сдвинутым `due_at` и записью `Rearmed`.
///
/// `WHERE` проверяет `state = 'pending' AND due_at <= now` вместе, атомарно
/// одним условным `UPDATE` — не отдельным чтением, а потом отдельной
/// записью. Без `due_at` в этом же условии два потребителя гонки на
/// ПОВТОРЯЮЩЕМСЯ напоминании расходятся не по правде, а по времени опроса:
/// потребитель A побеждает и тем же вызовом перевзводит строку в `Pending` с
/// `due_at` в будущем; потребитель B, вычитавший ту же строку ДО перевзвода
/// A, увидит `state = 'pending'` и без проверки срока тоже выиграет —
/// сегодня такого второго потребителя нет, но демон второй волны становится
/// им (`aurelius:reminders:mark-delivered-race`).
pub fn mark_delivered(conn: &Connection, id: &str, via: &str, now: DateTime<Utc>) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE reminders
            SET state = 'delivered', delivered_at = ?1, delivered_via = ?2,
                delivered_count = delivered_count + 1
          WHERE id = ?3 AND state = 'pending' AND due_at <= ?1",
        params![now.timestamp(), via, id],
    )?;
    if changed == 0 {
        return Ok(false);
    }
    write_event(conn, id, now, EventKind::Delivered, Some(via))?;

    let (due_at_secs, repeat_spec): (i64, Option<String>) = conn.query_row(
        "SELECT due_at, repeat_spec FROM reminders WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if let Some(delay) = repeat_spec.as_deref().and_then(parse_delay) {
        let next = next_occurrence(from_unix(due_at_secs), delay, now);
        conn.execute(
            "UPDATE reminders
                SET state = 'pending', due_at = ?1, delivered_at = NULL, delivered_via = NULL
              WHERE id = ?2",
            params![next.timestamp(), id],
        )?;
        write_event(
            conn,
            id,
            now,
            EventKind::Rearmed,
            Some(&format!("next {}", next.to_rfc3339())),
        )?;
    }
    Ok(true)
}

/// Перенос: `Pending`/`Delivered` → `Pending` с новым `due_at`,
/// `snooze_count` растёт, `original_due_at` не трогается — это и есть
/// честная память о том, что было обещано изначально. `Done`/`Cancelled`
/// перенести нельзя — вызов на терминальной строке отвечает `false`, а не
/// переписывает историю.
pub fn snooze(
    conn: &Connection,
    id: &str,
    until: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<bool> {
    let before: Option<i64> = conn
        .query_row(
            "SELECT due_at FROM reminders WHERE id = ?1 AND state IN ('pending', 'delivered')",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(before_secs) = before else {
        return Ok(false);
    };
    let changed = conn.execute(
        "UPDATE reminders
            SET state = 'pending', due_at = ?1, delivered_at = NULL, delivered_via = NULL,
                snooze_count = snooze_count + 1
          WHERE id = ?2 AND state IN ('pending', 'delivered')",
        params![until.timestamp(), id],
    )?;
    if changed == 0 {
        return Ok(false);
    }
    let detail = format!(
        "{} -> {}",
        from_unix(before_secs).to_rfc3339(),
        until.to_rfc3339()
    );
    write_event(conn, id, now, EventKind::Snoozed, Some(&detail))?;
    Ok(true)
}

/// Любое нетерминальное состояние → `Done`, со штампом и записью в журнале.
/// Повторный вызов на уже закрытой строке — `false`, не переписывание
/// прошлого.
pub fn done(conn: &Connection, id: &str, now: DateTime<Utc>) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE reminders SET state = 'done', done_at = ?1
          WHERE id = ?2 AND state NOT IN ('done', 'cancelled')",
        params![now.timestamp(), id],
    )?;
    if changed == 0 {
        return Ok(false);
    }
    write_event(conn, id, now, EventKind::Done, None)?;
    Ok(true)
}

/// Симметрично [`done`], но в исход `Cancelled` — второй исход, ради
/// различения которого от `Done` всё это перечисление и заведено.
pub fn cancel(conn: &Connection, id: &str, now: DateTime<Utc>) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE reminders SET state = 'cancelled', cancelled_at = ?1
          WHERE id = ?2 AND state NOT IN ('done', 'cancelled')",
        params![now.timestamp(), id],
    )?;
    if changed == 0 {
        return Ok(false);
    }
    write_event(conn, id, now, EventKind::Cancelled, None)?;
    Ok(true)
}

/// Список напоминаний, по умолчанию — только нетерминальные, по возрастанию
/// `due_at`.
pub fn list(
    conn: &Connection,
    project: Option<&str>,
    state_filter: Option<State>,
    include_terminal: bool,
    limit: usize,
) -> Result<Vec<Reminder>> {
    let mut sql = format!("SELECT {REMINDER_COLUMNS} FROM reminders WHERE 1 = 1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(state) = state_filter {
        sql.push_str(" AND state = ?");
        args.push(Box::new(state.as_str()));
    } else if !include_terminal {
        sql.push_str(" AND state NOT IN ('done', 'cancelled')");
    }
    if let Some(project) = project {
        sql.push_str(" AND project = ?");
        args.push(Box::new(project.to_owned()));
    }
    sql.push_str(" ORDER BY due_at ASC LIMIT ?");
    args.push(Box::new(i64::try_from(limit).unwrap_or(i64::MAX)));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args), row_to_reminder)?
        .filter_map(std::result::Result::ok)
        .collect();
    Ok(rows)
}

/// Журнал одного напоминания, от старой записи к новой — то, что делает
/// перенос видимым постфактум.
pub fn events(conn: &Connection, id: &str) -> Result<Vec<ReminderEvent>> {
    let mut stmt = conn.prepare(
        "SELECT at, kind, detail FROM reminder_events WHERE reminder_id = ?1 ORDER BY at ASC, id ASC",
    )?;
    let rows = stmt
        .query_map([id], |r| {
            let at: i64 = r.get(0)?;
            let kind_s: String = r.get(1)?;
            Ok(ReminderEvent {
                at: from_unix(at),
                kind: EventKind::parse(&kind_s).unwrap_or(EventKind::Created),
                detail: r.get(2)?,
            })
        })?
        .filter_map(std::result::Result::ok)
        .collect();
    Ok(rows)
}

/// Все напоминания одной задачи, по умолчанию — только нетерминальные.
pub fn for_task(conn: &Connection, task_id: &str, include_terminal: bool) -> Result<Vec<Reminder>> {
    let sql = if include_terminal {
        format!("SELECT {REMINDER_COLUMNS} FROM reminders WHERE task_id = ?1 ORDER BY due_at ASC")
    } else {
        format!(
            "SELECT {REMINDER_COLUMNS} FROM reminders WHERE task_id = ?1 \
             AND state NOT IN ('done', 'cancelled') ORDER BY due_at ASC"
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([task_id], row_to_reminder)?
        .filter_map(std::result::Result::ok)
        .collect();
    Ok(rows)
}

/// Полный id либо его уникальный префикс от 4 символов — не заставлять
/// набирать uuid целиком. Неоднозначный префикс, как и несуществующий,
/// разрешается в `None`: молчаливое «повезло и взяли не ту строку» здесь
/// дороже явного отказа.
pub fn resolve_id(conn: &Connection, prefix: &str) -> Result<Option<String>> {
    if prefix.chars().count() < 4 || !prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Ok(None);
    }
    let mut stmt = conn.prepare("SELECT id FROM reminders WHERE id LIKE ?1 || '%' LIMIT 2")?;
    let candidates: Vec<String> = stmt
        .query_map([prefix], |r| r.get(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(match candidates.len() {
        1 => candidates.into_iter().next(),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let dir = std::env::temp_dir().join(format!("aurelius-rem-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        crate::db::open(&dir.join("test.db")).expect("open test db")
    }

    #[test]
    fn parse_delay_accepts_the_four_suffixes_and_a_bare_number() {
        assert_eq!(parse_delay("5"), Some(Duration::minutes(5)));
        assert_eq!(parse_delay("5m"), Some(Duration::minutes(5)));
        assert_eq!(parse_delay("2h"), Some(Duration::hours(2)));
        assert_eq!(parse_delay("3d"), Some(Duration::days(3)));
        assert_eq!(parse_delay("1w"), Some(Duration::weeks(1)));
    }

    #[test]
    fn parse_delay_rejects_zero_negative_and_garbage() {
        assert_eq!(parse_delay("0"), None);
        assert_eq!(parse_delay("0m"), None);
        assert_eq!(parse_delay("-5m"), None);
        assert_eq!(parse_delay("garbage"), None);
        assert_eq!(parse_delay(""), None);
        assert_eq!(parse_delay("5x"), None);
    }

    #[test]
    fn parse_moment_sends_a_passed_hhmm_to_tomorrow() {
        // Момент за минуту до "сейчас" по локальному времени — гарантированно
        // уже прошёл сегодня независимо от часового пояса машины, на которой
        // гоняется тест.
        let now = Utc::now();
        let one_minute_ago = (now.with_timezone(&Local) - Duration::minutes(1)).time();
        let spec = one_minute_ago.format("%H:%M").to_string();

        let got = parse_moment(&spec, now).expect("parses");

        let expected_date = (now.with_timezone(&Local) + Duration::days(1)).date_naive();
        assert_eq!(got.with_timezone(&Local).date_naive(), expected_date);
    }

    #[test]
    fn parse_moment_rejects_garbage() {
        let now = Utc::now();
        assert_eq!(parse_moment("not a moment", now), None);
    }

    fn new_reminder(text: &str, due_at: DateTime<Utc>, owner: Owner) -> NewReminder {
        NewReminder {
            text: text.to_owned(),
            due_at,
            owner,
            task_id: None,
            project: None,
            repeat_spec: None,
        }
    }

    #[test]
    fn due_ignores_done_cancelled_and_delivered_rows() {
        let conn = test_conn();
        let now = Utc::now();
        let past = now - Duration::minutes(5);

        let a = add(&conn, new_reminder("a", past, Owner::Both)).expect("add a");
        let b = add(&conn, new_reminder("b", past, Owner::Both)).expect("add b");
        let c = add(&conn, new_reminder("c", past, Owner::Both)).expect("add c");
        done(&conn, &a.id, now).expect("done");
        cancel(&conn, &b.id, now).expect("cancel");
        mark_delivered(&conn, &c.id, "test", now).expect("deliver");

        let found = due(&conn, now, None, 10).expect("due");
        assert!(
            found.is_empty(),
            "done/cancelled/delivered must not be due: {found:?}"
        );
    }

    #[test]
    fn due_with_ai_filter_returns_ai_and_both_but_not_me() {
        let conn = test_conn();
        let now = Utc::now();
        let past = now - Duration::minutes(5);

        add(&conn, new_reminder("for me", past, Owner::Me)).expect("add me");
        add(&conn, new_reminder("for ai", past, Owner::Ai)).expect("add ai");
        add(&conn, new_reminder("for both", past, Owner::Both)).expect("add both");

        let found = due(&conn, now, Some(Owner::Ai), 10).expect("due");
        let texts: Vec<&str> = found.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts.len(), 2);
        assert!(texts.contains(&"for ai"));
        assert!(texts.contains(&"for both"));
        assert!(!texts.contains(&"for me"));
    }

    #[test]
    fn mark_delivered_is_won_by_exactly_one_of_two_calls() {
        let conn = test_conn();
        let now = Utc::now();
        let r = add(
            &conn,
            new_reminder("race", now - Duration::minutes(1), Owner::Both),
        )
        .expect("add");

        let first = mark_delivered(&conn, &r.id, "session-a", now).expect("first");
        let second = mark_delivered(&conn, &r.id, "session-b", now).expect("second");

        assert!(first, "first caller must win");
        assert!(
            !second,
            "second caller must lose — the row already left Pending"
        );
    }

    /// Регрессия на гонку двух потребителей одного ПОВТОРЯЮЩЕГОСЯ
    /// напоминания (`aurelius:reminders:mark-delivered-race`): пока
    /// `WHERE` проверял только `state`, потребитель B — вычитавший строку
    /// ДО того, как потребитель A перевзвёл её в будущее тем же вызовом —
    /// тоже проходил условие, потому что `state` к тому моменту снова было
    /// `pending`. Один живой потребитель этого не видел никогда; демон
    /// второй волны становится вторым, и без `due_at` в том же `WHERE` эта
    /// гонка стреляет по построению, а не по случайности.
    #[test]
    fn mark_delivered_two_consumer_race_on_a_repeating_reminder_is_won_once() {
        let conn = test_conn();
        let now = Utc::now();
        let mut r = new_reminder("standup", now - Duration::minutes(1), Owner::Both);
        r.repeat_spec = Some("1h".to_owned());
        let r = add(&conn, r).expect("add");

        // Consumer A takes it and, inside the SAME call, re-arms it
        // strictly into the future.
        let a = mark_delivered(&conn, &r.id, "consumer-a", now).expect("consumer a");
        assert!(a, "consumer A must win the still-due row");

        // Consumer B holds the row as it read it before A ran — same `now`,
        // the shape of a daemon tick racing the session hook. Its `UPDATE`
        // must fail on `due_at`, not slip through on `state` alone: A's
        // re-arm already put the row back into `Pending`, but for the NEXT
        // occurrence, not the one B saw.
        let b = mark_delivered(&conn, &r.id, "consumer-b", now).expect("consumer b");
        assert!(
            !b,
            "consumer B must lose — the row is pending for a future occurrence, not the one B saw"
        );

        let got = get(&conn, &r.id).expect("get").expect("row exists");
        assert_eq!(
            got.delivered_count, 1,
            "exactly one delivery must be counted, not two"
        );

        let history = events(&conn, &r.id).expect("events");
        let delivered_events = history
            .iter()
            .filter(|e| e.kind == EventKind::Delivered)
            .count();
        assert_eq!(
            delivered_events, 1,
            "exactly one Delivered journal entry, not one per consumer"
        );
        let rearmed_events = history
            .iter()
            .filter(|e| e.kind == EventKind::Rearmed)
            .count();
        assert_eq!(
            rearmed_events, 1,
            "exactly one Rearmed journal entry, from consumer A's win alone"
        );
    }

    #[test]
    fn repeating_reminder_comes_back_pending_strictly_in_the_future() {
        let conn = test_conn();
        let now = Utc::now();
        let mut r = new_reminder("standup", now - Duration::minutes(1), Owner::Both);
        r.repeat_spec = Some("1h".to_owned());
        let r = add(&conn, r).expect("add");

        mark_delivered(&conn, &r.id, "session", now).expect("deliver");

        let got = get(&conn, &r.id).expect("get").expect("row exists");
        assert_eq!(
            got.state,
            State::Pending,
            "must be re-armed, not left Delivered"
        );
        assert!(
            got.due_at > now,
            "next occurrence must be strictly in the future"
        );
    }

    #[test]
    fn repeat_missed_for_many_periods_lands_on_a_single_future_occurrence() {
        let conn = test_conn();
        let now = Utc::now();
        let mut r = new_reminder("checkin", now - Duration::days(3), Owner::Both);
        r.repeat_spec = Some("1m".to_owned());
        let r = add(&conn, r).expect("add");

        mark_delivered(&conn, &r.id, "session", now).expect("deliver");

        let got = get(&conn, &r.id).expect("get").expect("row exists");
        assert!(got.due_at > now);
        assert!(
            got.due_at < now + Duration::minutes(2),
            "must land just past now, not far in the future: {}",
            got.due_at
        );
        let history = events(&conn, &r.id).expect("events");
        let rearmed = history
            .iter()
            .filter(|e| e.kind == EventKind::Rearmed)
            .count();
        assert_eq!(
            rearmed, 1,
            "one delivery must produce exactly one rearm, not one per missed period"
        );
    }

    #[test]
    fn snooze_increments_count_keeps_original_and_logs_event() {
        let conn = test_conn();
        let now = Utc::now();
        let original = now + Duration::hours(1);
        let r = add(&conn, new_reminder("call back", original, Owner::Both)).expect("add");

        let until = now + Duration::hours(3);
        let ok = snooze(&conn, &r.id, until, now).expect("snooze");
        assert!(ok);

        let got = get(&conn, &r.id).expect("get").expect("row exists");
        assert_eq!(got.snooze_count, 1);
        assert_eq!(got.original_due_at.timestamp(), original.timestamp());
        assert_eq!(got.due_at.timestamp(), until.timestamp());
        assert_eq!(got.state, State::Pending);

        let history = events(&conn, &r.id).expect("events");
        assert!(history.iter().any(|e| e.kind == EventKind::Snoozed));
    }

    #[test]
    fn done_and_cancel_are_distinguishable_in_state_and_journal() {
        let conn = test_conn();
        let now = Utc::now();
        let a = add(&conn, new_reminder("a", now, Owner::Both)).expect("add a");
        let b = add(&conn, new_reminder("b", now, Owner::Both)).expect("add b");

        assert!(done(&conn, &a.id, now).expect("done"));
        assert!(cancel(&conn, &b.id, now).expect("cancel"));

        let a = get(&conn, &a.id).expect("get").expect("row exists");
        let b = get(&conn, &b.id).expect("get").expect("row exists");
        assert_eq!(a.state, State::Done);
        assert!(a.done_at.is_some());
        assert!(a.cancelled_at.is_none());
        assert_eq!(b.state, State::Cancelled);
        assert!(b.cancelled_at.is_some());
        assert!(b.done_at.is_none());

        let a_events = events(&conn, &a.id).expect("events");
        let b_events = events(&conn, &b.id).expect("events");
        assert!(a_events.iter().any(|e| e.kind == EventKind::Done));
        assert!(b_events.iter().any(|e| e.kind == EventKind::Cancelled));
    }

    #[test]
    fn done_on_an_already_done_row_returns_false() {
        let conn = test_conn();
        let now = Utc::now();
        let r = add(&conn, new_reminder("once", now, Owner::Both)).expect("add");
        assert!(done(&conn, &r.id, now).expect("first done"));
        assert!(!done(&conn, &r.id, now).expect("second done"));
    }

    #[test]
    fn events_returns_the_whole_journal_in_order() {
        let conn = test_conn();
        let now = Utc::now();
        let r = add(
            &conn,
            new_reminder("chain", now - Duration::minutes(1), Owner::Both),
        )
        .expect("add");
        snooze(&conn, &r.id, now + Duration::hours(1), now).expect("snooze");
        done(&conn, &r.id, now + Duration::hours(1)).expect("done");

        let history = events(&conn, &r.id).expect("events");
        let kinds: Vec<EventKind> = history.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![EventKind::Created, EventKind::Snoozed, EventKind::Done]
        );
    }

    #[test]
    fn resolve_id_needs_at_least_four_chars_and_rejects_ambiguity() {
        let conn = test_conn();
        let now = Utc::now();
        let r = add(&conn, new_reminder("x", now, Owner::Both)).expect("add");

        assert_eq!(
            resolve_id(&conn, &r.id[..3]).expect("resolve"),
            None,
            "too short"
        );
        assert_eq!(
            resolve_id(&conn, &r.id[..8]).expect("resolve"),
            Some(r.id.clone()),
            "unique prefix resolves"
        );
        assert_eq!(
            resolve_id(&conn, &r.id).expect("resolve"),
            Some(r.id.clone()),
            "full id resolves"
        );
    }
}
