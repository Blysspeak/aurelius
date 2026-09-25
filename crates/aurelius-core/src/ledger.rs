//! Ступень 5+7 «Бит-и-Дело»: клиринг гроссбуха и банкротство-поглощение.
//!
//! Клиринг переводит вердикты судьи в единую валюту — биты — и списывает цену
//! показа (render_miss). GC ранжирует узлы по `α·bits + β·yield` из калиброванных
//! счетов (не по времени!) и банкротит слабейших: банкрот не удаляется, а
//! ПОГЛОЩАЕТСЯ сильнейшим связанным узлом — граф уплотняется вокруг
//! платёжеспособных хабов без LLM-суммаризации.

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;

use crate::differ::ReinforcedWindow;

/// Тариф бонуса за подтверждённый исход, в «битах». Обе валюты сходятся в одном
/// счёте, чтобы ранжирование было одномерным.
const YIELD_BONUS_BITS: i64 = 64;

/// Credit a yield bonus for each reinforce window closed in this judge run.
/// The caller passes only windows closed now (`JudgeStats::reinforced_windows`),
/// so a window is credited exactly once: nothing here rescans closed windows.
pub fn credit_reinforced(conn: &Connection, windows: &[ReinforcedWindow]) -> Result<()> {
    let now = Utc::now().timestamp();
    for w in windows {
        conn.execute(
            "INSERT INTO ledger (node_id, session_id, bits_delta, kind, at)
             VALUES (?1, ?2, ?3, 'yield_bonus', ?4)",
            rusqlite::params![w.node_id, w.session_id, YIELD_BONUS_BITS, now],
        )?;
    }
    Ok(())
}

/// Провести клиринг сессии: штраф render_miss за показанное-но-не-процитированное.
/// yield-бонусы кредитует `credit_reinforced`; earn по сжатию потока считается
/// отдельно (когортно) и здесь не дублируется.
///
/// Once-only: only render_log rows newer than this session's last render_miss
/// posting are charged, so a repeated clearing adds nothing.
pub fn clear_session(conn: &Connection, session_id: &str) -> Result<()> {
    let now = Utc::now().timestamp();

    // render_miss: показано в снапшоте, но не процитировано — место потрачено зря.
    let missed: Vec<(String, i64)> = {
        let mut stmt = conn.prepare(
            "SELECT node_id, bytes FROM render_log
              WHERE session_id = ?1 AND cited = 0
                AND at > COALESCE((SELECT MAX(at) FROM ledger
                                    WHERE session_id = ?1 AND kind = 'render_miss'), -1)",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        rows.filter_map(std::result::Result::ok).collect()
    };
    for (node_id, bytes) in &missed {
        conn.execute(
            "INSERT INTO ledger (node_id, session_id, bits_delta, kind, at)
             VALUES (?1, ?2, ?3, 'render_miss', ?4)",
            rusqlite::params![node_id, session_id, -(bytes * 8), now],
        )?;
    }
    Ok(())
}

/// Итоговая ценность узла в битах: сумма всех проводок гроссбуха + сюрприз его
/// дельт. Timestamps не участвуют — только измерения.
pub fn node_value_bits(conn: &Connection, node_id: &str) -> Result<i64> {
    let ledger: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(bits_delta), 0) FROM ledger WHERE node_id = ?1",
            [node_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let surprise: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(surprisal_bits), 0) FROM delta WHERE node_id = ?1",
            [node_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(ledger + surprise)
}

pub struct GcStats {
    pub scanned: usize,
    pub absorbed: usize,
}

/// Банкротство-поглощение: узлы с ценностью ниже порога и без подтверждённых
/// путей поглощаются сильнейшим соседом. Наследуются входящие рёбра; сам узел
/// помечается degrade_stage и receivership (обратимо — история в node_version).
pub fn bankrupt_and_absorb(conn: &Connection, min_value_bits: i64) -> Result<GcStats> {
    let now = Utc::now().timestamp();
    let mut stats = GcStats {
        scanned: 0,
        absorbed: 0,
    };

    // Кандидаты: узлы БЕЗ подтверждённых путей (confirms=0 везде), не проекты,
    // не уже обанкроченные. Ценность считаем по каждому.
    let candidates: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT n.id FROM nodes n
              WHERE n.deleted_at IS NULL AND n.degrade_stage = 0
                AND n.node_type NOT IN ('\"project\"', '\"user_fact\"', '\"skill\"')
                AND NOT EXISTS (SELECT 1 FROM pathways p WHERE p.node_id = n.id AND p.confirms > 0)
                AND NOT EXISTS (SELECT 1 FROM receivership r WHERE r.node_id = n.id)
              LIMIT 500",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.filter_map(std::result::Result::ok).collect()
    };

    for node_id in &candidates {
        stats.scanned += 1;
        if node_value_bits(conn, node_id)? >= min_value_bits {
            continue;
        }
        // Сильнейший сосед: узел с максимальной ценностью среди связанных.
        let neighbours: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT CASE WHEN from_id = ?1 THEN to_id ELSE from_id END
                   FROM edges WHERE (from_id = ?1 OR to_id = ?1) AND deleted_at IS NULL",
            )?;
            let rows = stmt.query_map([node_id], |r| r.get::<_, String>(0))?;
            rows.filter_map(std::result::Result::ok).collect()
        };
        let mut best: Option<(String, i64)> = None;
        for nb in &neighbours {
            if nb == node_id {
                continue;
            }
            let v = node_value_bits(conn, nb)?;
            if best.as_ref().is_none_or(|(_, bv)| v > *bv) {
                best = Some((nb.clone(), v));
            }
        }
        let Some((absorber, _)) = best else { continue };

        // Наследование входящих рёбер: перевесить на поглотителя (dedup гасит
        // самопетли и дубли через уникальный индекс edges).
        conn.execute(
            "UPDATE OR IGNORE edges SET to_id = ?1
              WHERE to_id = ?2 AND from_id != ?1 AND deleted_at IS NULL",
            rusqlite::params![absorber, node_id],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO receivership (node_id, absorbed_by, at) VALUES (?1, ?2, ?3)",
            rusqlite::params![node_id, absorber, now],
        )?;
        conn.execute(
            "UPDATE nodes SET degrade_stage = 1 WHERE id = ?1",
            [node_id],
        )?;
        stats.absorbed += 1;
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::models::{NodeType, Relation};

    fn test_conn() -> Connection {
        let dir =
            std::env::temp_dir().join(format!("aurelius-ledger-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        db::open(&dir.join("test.db")).expect("open test db")
    }

    fn ledger_rows(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM ledger", [], |r| r.get(0))
            .expect("count")
    }

    #[test]
    fn clearing_credits_reinforce_and_debits_render_miss() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO render_log (session_id, node_id, layer, bytes, cited, at)
             VALUES ('s1', 'n2', 'semantic', 100, 0, 1)",
            [],
        )
        .expect("render");
        let closed = [ReinforcedWindow {
            window_id: 1,
            node_id: "n1".into(),
            session_id: "s1".into(),
        }];
        credit_reinforced(&conn, &closed).expect("credit");
        clear_session(&conn, "s1").expect("clear");
        assert_eq!(node_value_bits(&conn, "n1").expect("v1"), YIELD_BONUS_BITS);
        assert_eq!(node_value_bits(&conn, "n2").expect("v2"), -800);
    }

    #[test]
    fn second_clearing_run_adds_no_ledger_rows() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO labile_window (node_id, session_id, snapshot_hash, opened_at, closed_at, verdict)
             VALUES ('n1', 's1', 'h', 1, 2, 'reinforce')",
            [],
        )
        .expect("window");
        conn.execute(
            "INSERT INTO render_log (session_id, node_id, layer, bytes, cited, at)
             VALUES ('s1', 'n2', 'semantic', 100, 0, 1)",
            [],
        )
        .expect("render");
        // Run 1: the judge closes nothing new (window was closed earlier), so the
        // already-closed reinforce window must not be re-credited.
        let stats = crate::differ::close_ripe_windows(&conn, 0).expect("judge 1");
        credit_reinforced(&conn, &stats.reinforced_windows).expect("credit 1");
        clear_session(&conn, "s1").expect("clear 1");
        let after_first = ledger_rows(&conn);
        assert_eq!(after_first, 1, "only the render_miss posting");
        let stats = crate::differ::close_ripe_windows(&conn, 0).expect("judge 2");
        credit_reinforced(&conn, &stats.reinforced_windows).expect("credit 2");
        clear_session(&conn, "s1").expect("clear 2");
        assert_eq!(ledger_rows(&conn), after_first);
    }

    #[test]
    fn bankrupt_absorbs_worthless_into_strongest_neighbour() {
        let conn = test_conn();
        let strong = crate::graph::add_node(
            &conn,
            NodeType::Solution,
            "[demo] сильное решение",
            Some("подтверждено"),
            "t",
            serde_json::json!({}),
        )
        .expect("strong");
        let weak = crate::graph::add_node(
            &conn,
            NodeType::Concept,
            "[demo] пустышка",
            Some("никто не читал"),
            "t",
            serde_json::json!({}),
        )
        .expect("weak");
        crate::graph::add_edge(&conn, weak.id, strong.id, Relation::RelatedTo, 1.0).expect("edge");
        // Дать сильному ценность, слабому — ноль.
        conn.execute(
            "INSERT INTO ledger (node_id, session_id, bits_delta, kind, at) VALUES (?1, 's', 1000, 'earn', 1)",
            [strong.id.to_string()],
        ).expect("credit");

        let stats = bankrupt_and_absorb(&conn, 1).expect("gc");
        assert!(stats.absorbed >= 1);
        let absorbed_by: String = conn
            .query_row(
                "SELECT absorbed_by FROM receivership WHERE node_id = ?1",
                [weak.id.to_string()],
                |r| r.get(0),
            )
            .expect("receivership");
        assert_eq!(absorbed_by, strong.id.to_string());
    }
}
