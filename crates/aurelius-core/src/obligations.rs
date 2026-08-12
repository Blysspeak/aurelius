//! Ступень 6 «Бит-и-Дело»: проспективный контур — память о будущем.
//!
//! Обязательство существует ⇔ его проводки не сбалансированы (не булев флаг,
//! а аудируемый журнал). «Напряжение» открытой проводки РАСТЁТ со временем
//! (инверсия кривой забывания по Зейгарник): недоделанное лезет наверх само,
//! пока детерминированный матчер не найдёт погашающее событие. Ненадёжный
//! контрагент копит напряжение быстрее — обратная связь исход→salience.

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

/// Маркеры комиссивов: обещание/долг. Грамматика намеренно простая и
/// расширяемая — детерминизм важнее полноты, ложное обязательство дороже.
const COMMISSIVES: &[&str] = &[
    "сделаю",
    "сделаю потом",
    "надо потом",
    "надо будет",
    "завтра",
    "позже",
    "допилю",
    "доделаю",
    "вернусь к",
    "напишу",
    "отпишу",
    "todo",
    "fixme",
    "обещаю",
    "займусь",
    "гляну",
    "посмотрю позже",
];

/// Отпечаток объекта обязательства: нормализованные значимые токены, склеенные
/// в строку. Хранится как есть (не хэш!) — по нему и дедуп открытых проводок,
/// и FTS-матч погашающего события по словам.
///
/// Отпечаток НЕ показывают человеку: токены отсортированы по алфавиту, и фраза
/// «добить клиринг гроссбуха» превращается в «гроссбуха добить клиринг». Для
/// показа есть `object_text` — исходное предложение.
fn object_fp(text: &str) -> String {
    let mut toks: Vec<String> = text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() > 3 && !COMMISSIVES.contains(t))
        .take(12)
        .map(str::to_owned)
        .collect();
    toks.sort();
    toks.dedup();
    toks.join(" ")
}

/// Служебные символы: в живой фразе редки, в команде и коде — на каждом шагу.
const CODEY: &[char] = &[
    '{', '}', '[', ']', '(', ')', '<', '>', '$', ';', '=', '|', '\\', '/', '"', '\'', '`', '*',
    '&', '^', '#', '@', '~', '+',
];
/// Доля служебных символов, выше которой текст считается не речью. Именно
/// ДОЛЯ, а не факт наличия: одна скобка в живой фразе её не дисквалифицирует.
const CODEY_MAX_RATIO: f64 = 0.03;
/// Речь коротка. Двухкилобайтная нагрузка следа — это вывод команды, а не
/// обещание, даже если обещание внутри упомянуто.
const UTTERANCE_MAX_CHARS: usize = 300;

/// Обязательство рождается только из РЕЧИ.
///
/// Полезная нагрузка следа — чаще всего команда или код. Строка
/// `au trace -m "надо потом добить клиринг"` УПОМИНАЕТ обещание, но сама
/// обещанием не является. Без этой заслонки такой след заводил обязательство,
/// и слой «Давление» показывал владельцу отпечаток вида
/// «aurelius blyss force foreach item local path remove silentlycontinue» —
/// шум ровно в том месте снапшота, которое обязано быть коротким и точным.
///
/// Детектор нарочно грубый и односторонний: фраза с URL или с длинной цитатой
/// в него не пройдёт. Это осознанный перекос — пропущенное обязательство стоит
/// дешевле выдуманного (см. шапку модуля).
pub fn is_utterance(text: &str) -> bool {
    let total = text.chars().count();
    if total == 0 || total > UTTERANCE_MAX_CHARS || text.lines().count() > 2 {
        return false;
    }
    let codey = text.chars().filter(|c| CODEY.contains(c)).count();
    #[allow(clippy::cast_precision_loss)]
    let ratio = codey as f64 / total as f64;
    ratio <= CODEY_MAX_RATIO
}

/// Комиссив ищется по границам слов, а не подстрокой: `contains("todo")`
/// срабатывал внутри `TodoWrite`, `contains("завтра")` — внутри имени файла.
fn find_commissive(low: &str) -> Option<&'static str> {
    COMMISSIVES
        .iter()
        .filter_map(|m| {
            let mut from = 0;
            while let Some(rel) = low[from..].find(*m) {
                let at = from + rel;
                let end = at + m.len();
                let left_free = !low[..at]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric);
                let right_free = !low[end..].chars().next().is_some_and(char::is_alphanumeric);
                if left_free && right_free {
                    return Some((*m, at));
                }
                from = end;
            }
            None
        })
        .min_by_key(|(_, at)| *at)
        .map(|(m, _)| m)
}

/// Предложение, в котором стоит комиссив, — читаемый объект обязательства.
/// Режем по границам предложений на ИСХОДНОМ тексте: индексы из lowercase
/// в оригинал не переносятся (регистр меняет длину в байтах у части символов).
fn promise_sentence(text: &str) -> Option<(&'static str, &str)> {
    text.split(['.', '!', '?', '\n', ';'])
        .find_map(|part| find_commissive(&part.to_lowercase()).map(|verb| (verb, part.trim())))
}

fn bump_profile(conn: &Connection, node: &str, field: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO counterparty_profile (node, opened, closed, breach)
         VALUES (?1, 0, 0, 0) ON CONFLICT(node) DO NOTHING",
        [node],
    )?;
    conn.execute(
        &format!("UPDATE counterparty_profile SET {field} = {field} + 1 WHERE node = ?1"),
        [node],
    )?;
    Ok(())
}

/// Сколько текста обещания храним для показа. Больше строки в снапшоте всё
/// равно не поместится.
const OBJECT_TEXT_CAP: usize = 160;

/// Разбор обещания: маркер и читаемый объект. Объект — ПРЕДЛОЖЕНИЕ с
/// комиссивом, а не весь текст: и отпечаток чище, и дедуп точнее (две разные
/// фразы в одном сообщении — два разных долга).
fn promise(text: &str) -> Option<(&'static str, String)> {
    if !is_utterance(text) {
        return None;
    }
    let (verb, sentence) = promise_sentence(text)?;
    let object: String = sentence.chars().take(OBJECT_TEXT_CAP).collect();
    // «Сделаю завтра» — обещание без объекта. Пустой отпечаток схлопнул бы под
    // уникальным индексом все такие фразы в одну строку, поэтому не заводим.
    if object_fp(&object).is_empty() {
        return None;
    }
    Some((verb, object))
}

/// Читаемый объект обещания, если текст вообще является обещанием. Публичный
/// ради миграции V12: тем же правилом она перепроверяет уже заведённое.
pub fn readable_object(text: &str) -> Option<String> {
    promise(text).map(|(_, object)| object)
}

/// Завести обязательство, если текст — речь и в нём есть комиссив. Дедуп по
/// (debtor, creditor, object_fp) среди открытых. Возвращает id или None.
pub fn intake(
    conn: &Connection,
    text: &str,
    debtor: &str,
    creditor: &str,
    src_trace: Option<i64>,
) -> Result<Option<i64>> {
    let Some((verb, object)) = promise(text) else {
        return Ok(None);
    };
    // Один след порождает обязательство ровно раз: Stop-цепочка гоняет окно
    // следов повторно, без этого закрытое обязательство перерождалось бы копией.
    if let Some(tr) = src_trace {
        let seen: Option<i64> = conn
            .query_row(
                "SELECT id FROM obligations WHERE src_trace = ?1",
                [tr],
                |r| r.get(0),
            )
            .ok();
        if let Some(id) = seen {
            return Ok(Some(id));
        }
    }
    let fp = object_fp(&object);
    let dup: Option<i64> = conn
        .query_row(
            "SELECT id FROM obligations
              WHERE debtor = ?1 AND creditor = ?2 AND object_fp = ?3 AND closed_at IS NULL",
            [debtor, creditor, &fp],
            |r| r.get(0),
        )
        .ok();
    if dup.is_some() {
        return Ok(dup);
    }
    conn.execute(
        "INSERT INTO obligations
             (debtor, creditor, verb_class, object_fp, object_text, opened_at, src_trace)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            debtor,
            creditor,
            verb,
            fp,
            object,
            Utc::now().timestamp(),
            src_trace
        ],
    )?;
    bump_profile(conn, debtor, "opened")?;
    Ok(Some(conn.last_insert_rowid()))
}

fn sig_tokens(text: &str) -> std::collections::HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() > 3)
        .map(str::to_owned)
        .collect()
}

/// Погасить обязательства, чей объект РЕАЛЬНО совпадает с событием (коммит,
/// сообщение). FTS — дешёвый префильтр по одному общему слову; закрываем лишь
/// при существенном пересечении токенов (≥2 общих и ≥половины слов объекта),
/// иначе коммит про X гасил бы обещание про Y из-за общего имени проекта.
pub fn settle(conn: &Connection, event_text: &str, actor: &str) -> Result<usize> {
    let ev = sig_tokens(event_text);
    if ev.is_empty() {
        return Ok(0);
    }
    let fts: String = ev
        .iter()
        .take(8)
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" OR ");
    let candidates: Vec<(i64, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT o.id, o.debtor, o.object_fp FROM obligations_fts f
               JOIN obligations o ON o.id = f.rowid
              WHERE obligations_fts MATCH ?1 AND o.closed_at IS NULL AND o.debtor = ?2
              LIMIT 40",
        )?;
        let rows = stmt.query_map([&fts, &actor.to_owned()], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        rows.filter_map(std::result::Result::ok).collect()
    };
    let now = Utc::now().timestamp();
    let mut closed = 0;
    for (id, debtor, object_fp) in &candidates {
        let obj = sig_tokens(object_fp);
        let shared = obj.intersection(&ev).count();
        // Два общих слова — сигнал; одно (обычно имя проекта) гасило бы чужое.
        if shared < 2 {
            continue;
        }
        conn.execute(
            "UPDATE obligations SET closed_at = ?1 WHERE id = ?2",
            rusqlite::params![now, id],
        )?;
        bump_profile(conn, debtor, "closed")?;
        closed += 1;
    }
    Ok(closed)
}

pub struct OpenObligation {
    pub id: i64,
    pub debtor: String,
    pub creditor: String,
    /// Читаемый объект для показа, НЕ отпечаток.
    pub object: String,
    pub tension: f64,
}

/// Открытые обязательства по убыванию напряжения. Напряжение растёт с
/// возрастом и с breach_ratio контрагента (ленивый расчёт при чтении).
pub fn top_by_tension(conn: &Connection, limit: usize) -> Result<Vec<OpenObligation>> {
    let now = Utc::now().timestamp();
    // Строки до V12 читаемого текста не имеют — им остаётся отпечаток: он
    // некрасив, но это лучше пустой строки в снапшоте.
    let mut stmt = conn.prepare(
        "SELECT o.id, o.debtor, o.creditor,
                COALESCE(NULLIF(o.object_text, ''), o.object_fp), o.opened_at,
                COALESCE(p.opened, 0), COALESCE(p.breach, 0)
           FROM obligations o
           LEFT JOIN counterparty_profile p ON p.node = o.debtor
          WHERE o.closed_at IS NULL",
    )?;
    let mut rows: Vec<OpenObligation> = stmt
        .query_map([], |r| {
            let opened_at: i64 = r.get(4)?;
            let p_opened: i64 = r.get(5)?;
            let p_breach: i64 = r.get(6)?;
            #[allow(clippy::cast_precision_loss)]
            let age_days = (now - opened_at) as f64 / 86_400.0;
            let breach_ratio = if p_opened > 0 {
                p_breach as f64 / p_opened as f64
            } else {
                0.0
            };
            let tension = (age_days + 1.0) * (1.0 + breach_ratio);
            Ok(OpenObligation {
                id: r.get(0)?,
                debtor: r.get(1)?,
                creditor: r.get(2)?,
                object: r.get(3)?,
                tension,
            })
        })?
        .filter_map(std::result::Result::ok)
        .collect();
    rows.sort_by(|a, b| {
        b.tension
            .partial_cmp(&a.tension)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(limit);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn test_conn() -> Connection {
        let dir = std::env::temp_dir().join(format!("aurelius-ob-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        db::open(&dir.join("test.db")).expect("open test db")
    }

    #[test]
    fn intake_only_on_commissive_and_dedups() {
        let conn = test_conn();
        assert!(intake(&conn, "просто факт без обещания", "я", "влад", None)
            .expect("intake")
            .is_none());
        let a = intake(&conn, "допилю судью исхода завтра", "я", "влад", None).expect("a");
        let b = intake(&conn, "завтра допилю судью исхода", "я", "влад", None).expect("b");
        assert_eq!(a, b, "тот же объект — та же открытая проводка");
    }

    /// Настоящие нагрузки следов, заведшие мусор в слое «Давление» 12.08:
    /// команда лишь УПОМИНАЕТ обещание в своих аргументах.
    const SHELL_TRACE: &str = r#"$bin="C:\Users\blyss\.local\bin"; foreach ($n in "au","aurelius") { $t="$bin\$n.exe"; if (Test-Path $t) { Rename-Item $t "$n.exe.o5" -Force } }; au trace -k tool_call -m "надо потом добить клиринг гроссбуха aurelius" -s w4-e2e; au judge"#;
    const NODE_TRACE: &str = r#"node -e "const {DatabaseSync}=require('node:sqlite'); const db=new DatabaseSync(process.env.APPDATA+'/aurelius/aurelius.db'); db.prepare('DELETE FROM obligations').run()"
au trace -k tool_call -m "надо потом прикрутить телеметрию задержек к боту blyss""#;

    #[test]
    fn command_mentioning_a_promise_is_not_a_promise() {
        let conn = test_conn();
        for payload in [SHELL_TRACE, NODE_TRACE] {
            assert_eq!(readable_object(payload), None, "команда — не речь");
            assert!(intake(&conn, payload, "я", "влад", None)
                .expect("intake")
                .is_none());
        }
        // Короткая команда — тоже не речь: длину ловит не только потолок.
        assert_eq!(readable_object(r#"rm -rf /tmp/x; echo "надо потом""#), None);
    }

    #[test]
    fn object_is_a_readable_sentence_not_the_fingerprint() {
        let conn = test_conn();
        intake(
            &conn,
            "надо потом добить клиринг гроссбуха aurelius",
            "я",
            "влад",
            None,
        )
        .expect("intake");
        let top = top_by_tension(&conn, 5).expect("top");
        assert_eq!(
            top[0].object,
            "надо потом добить клиринг гроссбуха aurelius"
        );
        // Отпечаток тем временем остаётся отсортированным мешком — он для
        // дедупа, и именно поэтому его нельзя показывать.
        let fp: String = conn
            .query_row("SELECT object_fp FROM obligations", [], |r| r.get(0))
            .expect("fp");
        assert_ne!(fp, top[0].object);
    }

    #[test]
    fn commissive_must_be_a_whole_word() {
        let conn = test_conn();
        // «гляну» внутри «глянуть» и «todo» внутри «TodoWrite» — не комиссивы.
        assert_eq!(
            readable_object("вызов TodoWrite обновил список задач"),
            None
        );
        assert!(
            intake(&conn, "прогнал todolist по проекту", "я", "влад", None)
                .expect("intake")
                .is_none()
        );
    }

    #[test]
    fn promise_without_an_object_is_not_recorded() {
        let conn = test_conn();
        // Значимых слов не остаётся — под уникальным индексом такие фразы
        // схлопнулись бы в одну строку.
        assert!(intake(&conn, "сделаю завтра", "я", "влад", None)
            .expect("intake")
            .is_none());
    }

    #[test]
    fn same_trace_mints_once_even_after_close() {
        let conn = test_conn();
        let a = intake(&conn, "допилю сборщик мусора завтра", "я", "влад", Some(42)).expect("a");
        conn.execute(
            "UPDATE obligations SET closed_at = 1 WHERE id = ?1",
            [a.unwrap()],
        )
        .expect("close");
        // Тот же след повторно — не должен родить копию закрытого.
        let b = intake(&conn, "допилю сборщик мусора завтра", "я", "влад", Some(42)).expect("b");
        assert_eq!(a, b, "один след — одно обязательство навсегда");
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM obligations", [], |r| r.get(0))
            .expect("count");
        assert_eq!(total, 1);
    }

    #[test]
    fn settle_ignores_weakly_related_event() {
        let conn = test_conn();
        intake(
            &conn,
            "надо потом добавить латентность снапшота aurelius",
            "я",
            "влад",
            Some(1),
        )
        .expect("intake");
        // Коммит про ДРУГОЕ, общее слово только «aurelius» — не гасить.
        let closed = settle(&conn, "клиринг гроссбуха aurelius добит", "я").expect("settle");
        assert_eq!(closed, 0, "одного общего слова (имя проекта) мало");
    }

    #[test]
    fn settle_closes_matching_open() {
        let conn = test_conn();
        intake(
            &conn,
            "надо потом починить вебхуки paysido",
            "я",
            "влад",
            None,
        )
        .expect("intake");
        let closed = settle(&conn, "вебхуки paysido починены, ушли", "я").expect("settle");
        assert_eq!(closed, 1);
        let open: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM obligations WHERE closed_at IS NULL",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(open, 0);
    }

    #[test]
    fn tension_rewards_unreliable_counterparty() {
        let conn = test_conn();
        intake(&conn, "обещаю глянуть отчёт", "тео", "влад", None).expect("intake");
        // Тео однажды нарушил — breach поднимает напряжение.
        bump_profile(&conn, "тео", "opened").expect("p");
        bump_profile(&conn, "тео", "breach").expect("p");
        let top = top_by_tension(&conn, 5).expect("top");
        assert_eq!(top.len(), 1);
        assert!(top[0].tension > 1.0);
    }
}
