//! `au eval-search` — recall@5, recall@10 и MRR@10 эталонного набора
//! (`fixtures/eval/search-baseline.jsonl`) по трём движкам: FTS, dense, RRF.
//!
//! Отдельная подкоманда, а не ещё один вид кейса `au eval`: тот меряет
//! замороженную базу по sha256, а эталон поиска меряет базу, в которой лежат
//! векторы корпуса, — живую или названную `--db`. Вектор запроса берётся у
//! резидентного демона; демона нет — dense и RRF пропускаются с причиной,
//! а FTS считается всё равно.

use anyhow::Result;
use aurelius_core::graph;
use aurelius_core::search_eval::{self, Board, DEPTH};
use std::path::PathBuf;

const DEFAULT_CASES: &str = "fixtures/eval/search-baseline.jsonl";
const CLASSES: [&str; 5] = ["ru", "en", "cross", "key", "all"];

pub async fn run(cases: Option<String>, db: Option<String>, json_out: bool) -> Result<()> {
    let cases_path = cases.map_or_else(
        || crate::commands::eval_from_repo_root(DEFAULT_CASES),
        PathBuf::from,
    );
    let case_list = search_eval::load(&cases_path)?;
    let db_file = db.map_or_else(aurelius_core::db_path, PathBuf::from);
    let conn = aurelius_core::db::open_readonly(&db_file)?;
    let home = crate::commands::embed_socket_home();

    let has_vectors: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = 'node_embeddings')",
        [],
        |row| row.get(0),
    )?;
    let mut board = Board::default();
    let mut skipped: Option<String> =
        (!has_vectors).then(|| "в базе нет таблицы node_embeddings".to_owned());

    for case in &case_list {
        let fts = graph::search_ranked(&conn, &case.query, DEPTH)?.nodes;
        let ids = |nodes: &[aurelius_core::models::Node]| -> Vec<uuid::Uuid> {
            nodes.iter().map(|n| n.id).collect()
        };
        board.add(
            "fts",
            &case.class,
            search_eval::score(&ids(&fts), &case.expect),
        );
        if skipped.is_some() {
            continue;
        }
        let (vector, notice) = graph::query_vector_for_search(&home, &case.query).await;
        let Some(vector) = vector else {
            skipped = notice.or_else(|| Some("демон эмбеддингов не ответил".to_owned()));
            continue;
        };
        let dense = graph::dense_search(&conn, &vector, DEPTH)?;
        board.add(
            "dense",
            &case.class,
            search_eval::score(&ids(&dense), &case.expect),
        );
        let (fused, _) = graph::hybrid_seeds(&conn, &case.query, &vector, DEPTH)?;
        board.add(
            "rrf",
            &case.class,
            search_eval::score(&ids(&fused), &case.expect),
        );
    }
    // Частичный векторный прогон несравним с полным: демон упал посреди —
    // выбрасываем обе векторные строки целиком, а не печатаем долю от части.
    if skipped.is_some() {
        board.0.remove("dense");
        board.0.remove("rrf");
    }

    if json_out {
        let out = serde_json::json!({
            "cases": cases_path.display().to_string(),
            "db": db_file.display().to_string(),
            "depth": DEPTH,
            "board": board,
            "vector_skipped": skipped,
        });
        println!("{out}");
        return Ok(());
    }
    println!(
        "эталон {} ({} кейсов), база {}",
        cases_path.display(),
        case_list.len(),
        db_file.display()
    );
    println!(
        "{:<6} {:<6} {:>3} {:>7} {:>7} {:>6}",
        "движок", "класс", "n", "R@5", "R@10", "MRR"
    );
    for (engine, row) in &board.0 {
        for class in CLASSES {
            if let Some(t) = row.get(class) {
                let pct = |k: usize| 100.0 * k as f64 / t.n.max(1) as f64;
                println!(
                    "{engine:<6} {class:<6} {:>3} {:>6.1}% {:>6.1}% {:>6.3}",
                    t.n,
                    pct(t.at5),
                    pct(t.at10),
                    t.mrr()
                );
            }
        }
    }
    if let Some(reason) = skipped {
        println!("dense и rrf пропущены: {reason}");
    }
    Ok(())
}
