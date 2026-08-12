//! Ступень 2 «Бит-и-Дело», шлюз сюрприза: сколько ИНФОРМАЦИИ несёт запись.
//!
//! Текст сжимается дважды: против обученного zstd-словаря своего scope
//! («ожидание» системы) и без словаря. NCS = C(x|dict)/C(x|null): близко к
//! нулю — запись предсказуема, ничего нового; близко к единице — сюрприз.
//! Никаких LLM и эмбеддингов: ценность меряется компрессором.
//!
//! Волна 2 работает в advisory-режиме: NCS и surprisal записываются в
//! дельта-счёт, но ничего не отбрасывают — порог подбирается по живым данным.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};

/// Минимум образцов, после которого словарь scope имеет смысл обучать.
const MIN_SAMPLES: usize = 32;
/// Размер словаря: 16К хватает для заметок, а обучение остаётся мгновенным.
const DICT_CAP: usize = 16 * 1024;
const LEVEL: i32 = 9;

pub struct Surprise {
    pub raw_len: usize,
    pub resid_len: usize,
    pub surprisal_bits: i64,
    /// C(x|dict)/C(x|null); 1.0, если словаря у scope ещё нет.
    pub ncs: f64,
    pub epoch: i64,
}

fn latest_dict(conn: &Connection, scope: &str) -> Result<Option<(i64, Vec<u8>)>> {
    Ok(conn
        .query_row(
            "SELECT epoch, blob FROM codec WHERE scope = ?1 ORDER BY epoch DESC LIMIT 1",
            [scope],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)),
        )
        .optional()?)
}

/// Измерить сюрприз текста против ожидания scope. Чистое измерение без записи.
pub fn measure(conn: &Connection, scope: &str, text: &str) -> Result<Surprise> {
    let raw = text.as_bytes();
    let null_len = zstd::bulk::compress(raw, LEVEL)
        .context("zstd compress without dictionary")?
        .len();
    let (epoch, resid_len, ncs) = match latest_dict(conn, scope)? {
        Some((epoch, dict)) => {
            let mut c = zstd::bulk::Compressor::with_dictionary(LEVEL, &dict)
                .context("zstd compressor with dictionary")?;
            let with_dict = c
                .compress(raw)
                .context("zstd compress with dictionary")?
                .len();
            #[allow(clippy::cast_precision_loss)]
            let ncs = if null_len == 0 {
                1.0
            } else {
                with_dict as f64 / null_len as f64
            };
            (epoch, with_dict, ncs.min(1.5))
        }
        None => (0, null_len, 1.0),
    };
    Ok(Surprise {
        raw_len: raw.len(),
        resid_len,
        surprisal_bits: (resid_len as i64) * 8,
        ncs,
        epoch,
    })
}

/// Измерить и открыть дельта-счёт узла. Возвращает измерение.
pub fn record(conn: &Connection, node_id: &str, scope: &str, text: &str) -> Result<Surprise> {
    let s = measure(conn, scope, text)?;
    conn.execute(
        "INSERT INTO delta (node_id, scope, raw_len, resid_len, surprisal_bits, ncs, epoch_born)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            node_id,
            scope,
            s.raw_len as i64,
            s.resid_len as i64,
            s.surprisal_bits,
            s.ncs,
            s.epoch,
        ],
    )?;
    Ok(s)
}

/// Обучить словарь ожиданий scope на заметках его узлов. Возвращает номер
/// эпохи или None, если образцов ещё мало. Старые эпохи не удаляются —
/// дельты помнят, против какой эпохи рождались.
pub fn train(conn: &Connection, scope: &str) -> Result<Option<i64>> {
    let mut stmt = conn.prepare(
        "SELECT note FROM nodes
          WHERE note IS NOT NULL AND deleted_at IS NULL
            AND (label LIKE '[' || ?1 || ']%' OR ?1 = 'global')
          ORDER BY updated_at DESC LIMIT 500",
    )?;
    let samples: Vec<Vec<u8>> = stmt
        .query_map([scope], |r| r.get::<_, String>(0))?
        .filter_map(std::result::Result::ok)
        .map(String::into_bytes)
        .collect();
    if samples.len() < MIN_SAMPLES {
        return Ok(None);
    }
    let dict = zstd::dict::from_samples(&samples, DICT_CAP).context("ZDICT training")?;
    let next_epoch: i64 = conn.query_row(
        "SELECT COALESCE(MAX(epoch), 0) + 1 FROM codec WHERE scope = ?1",
        [scope],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO codec (scope, epoch, blob, trained_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![scope, next_epoch, dict, Utc::now().timestamp()],
    )?;
    Ok(Some(next_epoch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn test_conn() -> Connection {
        let dir =
            std::env::temp_dir().join(format!("aurelius-codec-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        db::open(&dir.join("test.db")).expect("open test db")
    }

    #[test]
    fn measure_without_dict_is_neutral() {
        let conn = test_conn();
        let s = measure(&conn, "global", "совершенно новый текст про платежи").expect("measure");
        assert!((s.ncs - 1.0).abs() < f64::EPSILON);
        assert!(s.surprisal_bits > 0);
        assert_eq!(s.epoch, 0);
    }

    #[test]
    fn trained_dict_lowers_ncs_for_familiar_text() {
        let conn = test_conn();
        // Корпус из однообразных заметок — словарь выучит их лексику.
        for i in 0..40 {
            crate::graph::add_node(
                &conn,
                crate::models::NodeType::Decision,
                &format!("[demo] решение {i}"),
                Some("вебхуки мерчанта уходят после подтверждения оплаты через processCallback"),
                "test",
                serde_json::json!({}),
            )
            .expect("node");
        }
        let epoch = train(&conn, "demo")
            .expect("train")
            .expect("enough samples");
        assert_eq!(epoch, 1);
        let familiar = measure(
            &conn,
            "demo",
            "вебхуки мерчанта уходят после подтверждения оплаты",
        )
        .expect("measure");
        let alien = measure(
            &conn,
            "demo",
            "квантовая хромодинамика глюонных струй на коллайдере",
        )
        .expect("measure");
        assert!(
            familiar.ncs < alien.ncs,
            "знакомый текст обязан сжиматься лучше: {} vs {}",
            familiar.ncs,
            alien.ncs
        );
    }

    #[test]
    fn record_opens_delta_account() {
        let conn = test_conn();
        record(&conn, "node-1", "global", "текст").expect("record");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM delta WHERE node_id = 'node-1'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(n, 1);
    }
}
