//! Эталон качества поиска (T012 спеки 011): known-item набор запросов с
//! ожидаемыми id узлов и метрики recall@5, recall@10, MRR@10 по классам
//! запросов (ru/en/cross/key); фикстура `fixtures/eval/search-baseline.jsonl`.
//! Векторов в ней нет: вектор запроса даёт демон в момент прогона, векторы
//! корпуса лежат в измеряемой базе; нет демона — dense пропускается.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Глубина выдачи, по которой считаются все метрики.
pub const DEPTH: usize = 10;

#[derive(Debug, Clone, Deserialize)]
pub struct SearchCase {
    pub id: String,
    pub class: String,
    pub query: String,
    pub expect: Vec<Uuid>,
    /// Как выведена пара запрос → ответ; читает человек, не код.
    #[serde(default)]
    pub how: String,
}

/// Кейсы файла; строка `meta` пропускается, пустые строки тоже.
///
/// # Errors
/// Файл не читается, строка не разбирается или кейс без ожиданий.
pub fn load(path: &Path) -> Result<Vec<SearchCase>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("не читается {}", path.display()))?;
    let mut cases = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with("{\"meta\"") {
            continue;
        }
        let case: SearchCase = serde_json::from_str(line)
            .with_context(|| format!("{}:{}: кейс не разбирается", path.display(), n + 1))?;
        if case.expect.is_empty() {
            bail!(
                "{}:{}: кейс {} без ожиданий",
                path.display(),
                n + 1,
                case.id
            );
        }
        cases.push(case);
    }
    Ok(cases)
}

/// Вклад запроса: попадание в топ-5, в топ-10 и 1/позиция первого попадания.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Hit {
    pub at5: bool,
    pub at10: bool,
    pub rr: f64,
}

#[must_use]
pub fn score(ranked: &[Uuid], expect: &[Uuid]) -> Hit {
    let first = ranked.iter().take(DEPTH).position(|id| expect.contains(id));
    match first {
        Some(pos) => Hit {
            at5: pos < 5,
            at10: true,
            rr: 1.0 / (pos as f64 + 1.0),
        },
        None => Hit::default(),
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Tally {
    pub n: usize,
    pub at5: usize,
    pub at10: usize,
    pub rr_sum: f64,
}

impl Tally {
    pub fn add(&mut self, hit: Hit) {
        self.n += 1;
        self.at5 += usize::from(hit.at5);
        self.at10 += usize::from(hit.at10);
        self.rr_sum += hit.rr;
    }

    #[must_use]
    pub fn mrr(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            self.rr_sum / self.n as f64
        }
    }
}

/// Таблица движок → класс → сумма; класс `all` — итог по движку.
#[derive(Debug, Default, Serialize)]
pub struct Board(pub BTreeMap<String, BTreeMap<String, Tally>>);

impl Board {
    pub fn add(&mut self, engine: &str, class: &str, hit: Hit) {
        let row = self.0.entry(engine.to_owned()).or_default();
        row.entry(class.to_owned()).or_default().add(hit);
        row.entry("all".to_owned()).or_default().add(hit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_counts_first_hit_position() {
        let ids: Vec<Uuid> = (0..12).map(|_| Uuid::new_v4()).collect();
        let hit = score(&ids, &[ids[6], ids[11]]);
        assert_eq!((hit.at5, hit.at10), (false, true));
        assert!((hit.rr - 1.0 / 7.0).abs() < 1e-12);
        assert_eq!(
            score(&ids, &[ids[11]]),
            Hit::default(),
            "за глубиной 10 — промах"
        );
        assert!(score(&ids, &[ids[0]]).at5);
    }

    #[test]
    fn frozen_fixture_is_balanced_and_clean() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/eval/search-baseline.jsonl");
        let cases = load(&path).unwrap();
        assert!((40..=60).contains(&cases.len()), "{}", cases.len());
        for class in ["ru", "en", "cross", "key"] {
            let n = cases.iter().filter(|c| c.class == class).count();
            assert!(n >= 10, "класс {class}: {n}");
        }
        for c in &cases {
            assert!(
                crate::secret::scan_text_for_lookalike(&c.query).is_none(),
                "{} похож на секрет",
                c.id
            );
        }
    }

    /// The held-out set: 10 cases per class, none aimed at a node the
    /// baseline already targets, so it measures what the baseline tuned.
    #[test]
    fn holdout_fixture_is_balanced_disjoint_and_clean() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/eval");
        let base = load(&dir.join("search-baseline.jsonl")).unwrap();
        let held = load(&dir.join("search-holdout.jsonl")).unwrap();
        assert_eq!(held.len(), 40);
        for class in ["ru", "en", "cross", "key"] {
            let n = held.iter().filter(|c| c.class == class).count();
            assert_eq!(n, 10, "класс {class}");
        }
        let targeted: std::collections::HashSet<Uuid> =
            base.iter().flat_map(|c| c.expect.iter().copied()).collect();
        for c in &held {
            assert!(
                c.expect.iter().all(|id| !targeted.contains(id)),
                "{} целит в узел эталона",
                c.id
            );
            assert!(
                crate::secret::scan_text_for_lookalike(&c.query).is_none(),
                "{} похож на секрет",
                c.id
            );
        }
    }
}
