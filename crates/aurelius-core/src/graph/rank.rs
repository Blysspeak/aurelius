//! Честный ранг recall: `score = r · P · R · A · T`, ровно пять множителей
//! (FR-007, `specs/010-waking-memory/data-model.md` §1).
//!
//! Все числа формулы живут в одном месте — [`RankWeights`] — чтобы `au eval`
//! (фаза F) калибровал их правкой одной структуры, а не охотой за константами
//! по трём крейтам. Все поля помечены как стартовые догадки: `Default`
//! несёт то, что записано в плане, `au eval` двигает эти числа замерами, а не
//! чтением кода заново.
//!
//! Момент времени — параметр [`score`], а не `Utc::now()` внутри модуля
//! (FR-030, D1): `R` зависит от даты прогона, и без внешнего момента один и
//! тот же кейс на замороженной фикстуре давал бы разные числа через неделю —
//! прямо против FR-025 (детерминизм `au eval`).

use crate::models::{Node, NodeType};
use crate::provenance::{Confidence, Provenance};
use chrono::{DateTime, Utc};

/// Множители честного ранга. Ровно пять входят в [`score`] — связности в этой
/// структуре нет, и параметра связности в `score()` тоже нет (D8, FR-007a):
/// `subgraph_degree` (`graph/mod.rs:145`) решает видимость и состав обхода,
/// а не порядок, и в произведение не входит.
#[derive(Debug, Clone, Copy)]
pub struct RankWeights {
    // --- P: провенанс, читается из `Option<Confidence>` напрямую ----------
    // `Provenance::confidence_or_default()` (`provenance.rs:311`) звать
    // отсюда запрещено: она схлопывает «поля `confidence` нет» (76% базы) и
    // явное `unverified` в одно значение (D7), а промежуток между ними и
    // есть разница между `p_absent` и `p_unverified` ниже.
    /// Стартовое, калибруется `au eval`.
    pub p_measured: f64,
    /// Стартовое, калибруется `au eval`.
    pub p_reported: f64,
    /// Стартовое, калибруется `au eval`.
    pub p_inferred: f64,
    /// Стартовое, калибруется `au eval`. Поля `confidence` в `data` нет.
    pub p_absent: f64,
    /// Стартовое, калибруется `au eval`. Поле есть и равно `"unverified"`.
    pub p_unverified: f64,

    // --- R: свежесть, R = 1 / (1 + age_days / freshness_scale_days) -------
    /// Стартовое, калибруется `au eval`. Было 90 — арифметика ниже (D5).
    pub freshness_scale_days: f64,

    // --- A: обращения, A = 1 + access_gain * min(ac, access_cap)/access_cap
    /// Стартовое, калибруется `au eval`.
    pub access_gain: f64,
    /// Стартовое, калибруется `au eval`.
    pub access_cap: i64,

    // --- T: множители типа узла, функция match, а не HashMap --------------
    // `NodeType` (`models.rs:5`) не выводит ни `Eq`, ни `Hash` — ключом карты
    // быть не может, это не стилистика, а ошибка компиляции.
    /// Стартовое, калибруется `au eval`.
    pub t_decision: f64,
    /// Стартовое, калибруется `au eval`.
    pub t_solution: f64,
    /// Стартовое, калибруется `au eval`.
    pub t_concept: f64,
    /// Стартовое, калибруется `au eval`.
    pub t_problem: f64,
    /// Стартовое, калибруется `au eval`.
    pub t_task: f64,
    /// Стартовое, калибруется `au eval`.
    pub t_file: f64,
    /// Стартовое, калибруется `au eval`. `Session`, `WorkLog`,
    /// `Custom("run")`. Было 0.3 — арифметика ниже (D6).
    pub t_machine: f64,
    /// Стартовое, калибруется `au eval`. Нейтральный вес: тип, которого нет
    /// в списке FR-007b. Совпадает с `p_absent` не случайно — та же мысль:
    /// «про этот узел ничего не сказано» не должно ни награждать, ни
    /// наказывать.
    pub t_other: f64,

    // --- r: узлы, которых FTS не оценивал -----------------------------------
    /// Стартовое, калибруется `au eval`. Узел пришёл обходом, а не посевом —
    /// ставится ровно на медиану посева, см. [`normalize_bm25`].
    pub r_traversed: f64,

    // --- ворота, а не множители ---------------------------------------------
    /// Стартовое, калибруется `au eval`. Терм более чем в N% узлов базы —
    /// один и тот же порог обслуживает FR-006 (отказ recall) и FR-015
    /// (терм не считается сущностью).
    pub df_veto_ratio: f64,
    /// Стартовое, калибруется `au eval`.
    pub min_significant_terms: usize,
    /// Стартовое, калибруется `au eval`. Порог истории 3b.
    pub fact_floor: f64,
    /// Стартовое, калибруется `au eval`.
    pub fact_lead_ratio: f64,
}

impl Default for RankWeights {
    fn default() -> Self {
        Self {
            p_measured: 1.0,
            p_reported: 0.8,
            p_inferred: 0.75,
            p_absent: 0.7,
            p_unverified: 0.6,

            freshness_scale_days: 180.0,

            access_gain: 0.15,
            access_cap: 20,

            t_decision: 1.0,
            t_solution: 1.0,
            t_concept: 0.95,
            t_problem: 0.9,
            t_task: 0.85,
            t_file: 0.5,
            t_machine: 0.15,
            t_other: 0.7,

            r_traversed: 0.5,

            df_veto_ratio: 0.01,
            min_significant_terms: 2,
            fact_floor: 0.5,
            fact_lead_ratio: 1.5,
        }
    }
}

/// Единственное место, где тип узла превращается в число. Ни одна ветка не
/// падает в `unreachable!`: тип, которого нет в списке FR-007b, получает
/// нейтральный `t_other`, а не вес соседа по алфавиту.
///
/// `NodeType::Run` and the legacy `Custom(s) if s == "run"` both carry the
/// machine weight and both stay: `link_evidence_run` (`graph/mod.rs:57`)
/// writes `Run` now, but a row synced in from a peer still on the old build
/// can arrive stamped `Custom("run")` long after this crate stopped writing
/// that form, and a "delete the old branch later" plan never actually
/// happens. Drop either arm and those rows fall through to `_` — no
/// compile error, no test failure, just a quiet drop from the machine weight
/// to the neutral one.
pub fn type_weight(w: &RankWeights, t: &NodeType) -> f64 {
    match t {
        NodeType::Decision => w.t_decision,
        NodeType::Solution => w.t_solution,
        NodeType::Concept => w.t_concept,
        NodeType::Problem => w.t_problem,
        NodeType::Task => w.t_task,
        NodeType::File => w.t_file,
        NodeType::Session | NodeType::WorkLog | NodeType::Run => w.t_machine,
        NodeType::Custom(s) if s == "run" => w.t_machine,
        _ => w.t_other,
    }
}

/// Провенанс-множитель. Читает `Option<Confidence>` напрямую — «поля нет» и
/// явное `unverified` различимы на этом уровне (D7) и склеиваются только
/// `confidence_or_default()`, которую этот модуль не зовёт.
fn provenance_weight(w: &RankWeights, confidence: Option<Confidence>) -> f64 {
    match confidence {
        Some(Confidence::Measured) => w.p_measured,
        Some(Confidence::Reported) => w.p_reported,
        Some(Confidence::Inferred) => w.p_inferred,
        Some(Confidence::Unverified) => w.p_unverified,
        None => w.p_absent,
    }
}

/// Возраст факта в днях, дробный. Точка отсчёта — `measured_at`, когда он
/// записан (замер и есть момент, с которого утверждение стало известным
/// таким, какое оно есть), и `created_at` узла как откат, когда замера нет —
/// тот же откат, что уже несёт `Provenance::staleness` (`provenance.rs:330`,
/// параметр `fallback_at`): чужого узла без даты замера в базе большинство,
/// и для них дата создания — единственная дата, которая вообще есть.
fn age_days(node: &Node, now: DateTime<Utc>) -> f64 {
    let since = Provenance::from_data(&node.data)
        .measured_at
        .unwrap_or(node.created_at);
    let seconds = now.signed_duration_since(since).num_seconds() as f64;
    (seconds / 86_400.0).max(0.0)
}

/// R: свежесть. `1` при возрасте 0, монотонно убывает, нуля не достигает —
/// FR-013: затухание по времени понижает ранг, но не скрывает и не удаляет.
fn freshness(w: &RankWeights, age_days: f64) -> f64 {
    1.0 / (1.0 + age_days / w.freshness_scale_days)
}

/// A: число обращений. Единственный множитель, который больше единицы —
/// `access_count` заменяет собой прежнее аддитивное слагаемое
/// `- (n.access_count * 0.1)` из `ORDER BY` (`graph/search.rs:131`, FR-007),
/// а не складывается с ним.
fn access_multiplier(w: &RankWeights, access_count: i64) -> f64 {
    let capped = access_count.clamp(0, w.access_cap) as f64;
    1.0 + w.access_gain * capped / w.access_cap as f64
}

/// `score = r · P · R · A · T` — ровно пять множителей (FR-007), связности
/// среди них нет (FR-007a, D8). `now` входит параметром: `Utc::now()` внутри
/// этого модуля запрещён (FR-030, D1) — см. доккомментарий модуля.
pub fn score(w: &RankWeights, node: &Node, r: f64, now: DateTime<Utc>) -> f64 {
    let p = provenance_weight(w, Provenance::from_data(&node.data).confidence);
    let fresh = freshness(w, age_days(node, now));
    let access = access_multiplier(w, node.access_count);
    let t = type_weight(w, &node.node_type);
    r * p * fresh * access * t
}

/// Reciprocal Rank Fusion — слияние FTS5 top-50 и dense KNN top-50 в один
/// посев (спека 011, `data-model.md` §2).
///
/// `12`, а не `60` из спеки. При `k = 60` и пуле 50 ЛЮБОЙ узел из обоих
/// списков бьёт ЛЮБОЙ узел из одного: худший двойной `2/(60+50) ≈ 0,0182`
/// больше лучшего одиночного `1/(60+1) ≈ 0,0164`. На кросс-язычном запросе
/// FTS цели не видит вовсе, а русские узлы, случайно совпавшие словами и
/// лежащие где-то в хвосте dense, выталкивали dense-первое место за десятку
/// (задача f12702e1). Малое `k` возвращает вес позиции. Замер 25.09.2026
/// (`au eval-search`, живая база, вместе с [`SCRIPT_BLIND_SHARE`] = 0,4):
/// k = 10…18 держит ru ≥ 92,9, en 100, cross 78,6; k = 8 роняет ru до 85,7,
/// k = 20 роняет cross до 57,1.
pub const RRF_K: f64 = 12.0;

/// Доля dense-вклада, которую узел получает вместо FTS-вклада, когда FTS
/// видеть его не могла: узел написан другим письмом, чем запрос (кириллица
/// против латиницы), и общих слов у них нет по построению. Отсутствие в
/// FTS-списке у такого узла — не свидетельство против него, а слепота
/// движка; без замены он проигрывает любому узлу своего письма, которого
/// FTS задела хоть одним словом. Замер 25.09.2026 при `RRF_K = 12`: 0,3…0,5
/// держат все цели, 0,2 роняет cross до 71,4, 0 — до 50,0; при 1,0 (полная
/// замена) ru падает до 64,3 — английские узлы на русский запрос
/// получают двойной вес.
pub const SCRIPT_BLIND_SHARE: f64 = 0.4;

/// RRF-скор одного узла: сумма `1 / (k + ранг)` по спискам, где узел
/// нашёлся. Складываются РАНГИ (позиция с единицы), а не сырые релевантности
/// FTS5 и dense — поэтому нормализация между движками не нужна вовсе.
/// `None` — узел не входит в этот список, и вклад по нему просто ноль, а не
/// штраф за последнее место: движок, нашедший узел единственным, не наказан
/// вторым, который его не увидел.
///
/// `fts_blind` — FTS не могла найти узел по построению (другое письмо, см.
/// [`SCRIPT_BLIND_SHARE`]); тогда вместо нулевого FTS-вклада узел получает
/// долю своего dense-вклада. Если FTS узел всё-таки нашла, её ранг главнее.
#[must_use]
pub fn rrf_score(fts_rank: Option<usize>, dense_rank: Option<usize>, fts_blind: bool) -> f64 {
    let term = |rank: Option<usize>| rank.map_or(0.0, |r| 1.0 / (RRF_K + r as f64));
    let fts = match fts_rank {
        Some(_) => term(fts_rank),
        None if fts_blind => SCRIPT_BLIND_SHARE * term(dense_rank),
        None => 0.0,
    };
    fts + term(dense_rank)
}

/// Мин-макс нормировка: растягивает сырые значения на весь `[0, 1]` —
/// худшее из списка получает `0`, лучшее `1`, — независимо от того, насколько
/// узок исходный разброс.
///
/// Не [`normalize_bm25`]: та нормирует относительно медианы, что подходит
/// bm25 (разброс на порядки между хорошим и плохим совпадением), но топит
/// RRF-скор посевов (`search::hybrid_seeds`) в одну точку у `0.5`. Причина —
/// в прежней константе `RRF_K = 60`: она на порядок больше диапазона рангов
/// топ-12 (`1..=12`), поэтому `1/(k+1)` и `1/(k+12)` отличаются всего на
/// ~18%, и медианное отношение `a/(a+median)` на такой узкой кучке чисел
/// возвращает почти одно и то же значение всем. Тот самый провал,
/// перечисленный в задаче спеки 011 третьим: «normalisation that flattened
/// the fused score» — измерено на живой базе 16.09.2026: `normalize_bm25` на
/// RRF-скоре давало `r` в полосе 0,49…0,51 для всех двенадцати посевов
/// (неотличимо от `RankWeights::r_traversed = 0.5`), из девяти кросс-язычных
/// кейсов проходил один; `normalize_minmax` вместо неё — то же измерение,
/// подробности в отчёте агента волны 011.
///
/// Единственное значение в списке — весь список получает `0.5`: RRF-скор
/// внутри списка есть, а различать нечего, и середина шкалы этого не выдаёт
/// ни за высокий сигнал, ни за низкий.
#[must_use]
pub fn normalize_minmax(raw: &[f64]) -> Vec<f64> {
    if raw.is_empty() {
        return Vec::new();
    }
    let min = raw.iter().copied().fold(f64::INFINITY, f64::min);
    let max = raw.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = max - min;
    raw.iter()
        .map(|v| if span == 0.0 { 0.5 } else { (v - min) / span })
        .collect()
}

/// Нормированный bm25 внутри одной выдачи: `r = |rank| / (|rank| +
/// median|rank|)`. Вход — только посевы FTS; узлы, пришедшие обходом, сюда
/// не подаются вовсе — они получают фиксированный `RankWeights::r_traversed`,
/// который по построению нормировки равен медиане посева.
#[must_use]
pub fn normalize_bm25(raw: &[f64]) -> Vec<f64> {
    if raw.is_empty() {
        return Vec::new();
    }
    let mut abs_values: Vec<f64> = raw.iter().map(|v| v.abs()).collect();
    abs_values.sort_by(f64::total_cmp);
    let median = median_of_sorted(&abs_values);
    raw.iter()
        .map(|v| {
            let a = v.abs();
            let denom = a + median;
            if denom == 0.0 {
                0.0
            } else {
                a / denom
            }
        })
        .collect()
}

fn median_of_sorted(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MemoryKind;
    use chrono::{Duration, TimeZone};
    use serde_json::json;
    use uuid::Uuid;

    /// Узел с произвольными `node_type`/`data`/`created_at`/`access_count` и
    /// нейтральными значениями во всём остальном — единственное, что не
    /// участвует ни в одной формуле [`score`].
    fn node(
        node_type: NodeType,
        data: serde_json::Value,
        created_at: DateTime<Utc>,
        access_count: i64,
    ) -> Node {
        Node {
            id: Uuid::new_v4(),
            node_type,
            label: "test".to_owned(),
            note: None,
            source: "test".to_owned(),
            data,
            created_at,
            updated_at: created_at,
            memory_kind: MemoryKind::Semantic,
            last_accessed_at: created_at,
            access_count,
            content_hash: None,
            created_by: None,
            updated_by: None,
            deleted_at: None,
            sync_seq: None,
        }
    }

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 8, 12, 0, 0)
            .single()
            .expect("валидная дата фикстуры")
    }

    // --- score(): ровно пять множителей, связности нет (FR-007a, D8) -------
    //
    // Инвариант ОДНОСТОРОННИЙ: два узла с равными P/R/A/T/r обязаны дать
    // равный score — это сценарий приёмки US2 №4 в его нынешней редакции.
    // Обратную сторону («узел без ребра не показывается») проверять нельзя:
    // она неверна. Видимость и состав выдачи решают рёбра и потолки обхода
    // (`walk`, `traverse.rs:105-163`; `MAX_TRAVERSAL_NODES`,
    // `MAX_TRAVERSAL_DEPTH`), посевы из `search()` кладутся безусловно, и
    // `subgraph_degree` (`graph/mod.rs:145`) считается уже ПОСЛЕ обхода и
    // кормит только сегодняшний компаратор, который эта спека убирает (C1).
    // Кейса `au eval` про степень нет и заводить его не нужно: это юнит-тест
    // формулы, а не фикстурный кейс — два узла с равными пятью множителями и
    // разной степенью в реальной базе не встречаются.
    #[test]
    fn score_has_no_channel_for_subgraph_degree() {
        let w = RankWeights::default();
        let now = fixed_now();
        // "Разная степень в подграфе" здесь — разная личность узла (id,
        // label неявно через новый Uuid). У score() нет параметра
        // связности и Node не несёт поля степени: подставить её нечем, и
        // это то самое отсутствие канала, которое инвариант утверждает.
        let low_degree = node(NodeType::Decision, json!({}), now, 3);
        let high_degree = node(NodeType::Decision, json!({}), now, 3);
        assert_ne!(low_degree.id, high_degree.id, "узлы обязаны быть разными");
        assert_eq!(
            score(&w, &low_degree, 0.6, now),
            score(&w, &high_degree, 0.6, now)
        );
    }

    // --- P: p_absent ≠ p_unverified, различимы только чтением Option -------
    #[test]
    fn absent_confidence_and_explicit_unverified_score_differently() {
        let w = RankWeights::default();
        let now = fixed_now();
        let absent = node(NodeType::Concept, json!({}), now, 0);
        let unverified = node(
            NodeType::Concept,
            json!({"confidence": "unverified"}),
            now,
            0,
        );
        let s_absent = score(&w, &absent, 0.6, now);
        let s_unverified = score(&w, &unverified, 0.6, now);
        assert_ne!(
            s_absent, s_unverified,
            "confidence_or_default() схлопнул бы эти два случая в один"
        );
        assert!(
            s_absent > s_unverified,
            "p_absent (0.7) > p_unverified (0.6)"
        );
    }

    // --- P: порядок множителей провенанса невозрастающий --------------------
    #[test]
    fn provenance_weights_are_non_increasing() {
        let w = RankWeights::default();
        assert!(w.p_measured >= w.p_reported);
        assert!(w.p_reported >= w.p_inferred);
        assert!(w.p_inferred >= w.p_absent);
        assert!(w.p_absent >= w.p_unverified);
        for p in [
            w.p_measured,
            w.p_reported,
            w.p_inferred,
            w.p_absent,
            w.p_unverified,
        ] {
            assert!(p > 0.0 && p <= 1.0, "p_* ∈ (0, 1]: {p}");
        }
    }

    // --- R: 1 при возрасте 0, монотонно убывает, нуля не достигает (FR-013) -
    //
    // Проверяется через приватный `freshness()`, а не через `score()`: у
    // score ровно пять множителей, и P/A остались бы посторонними
    // переменными в тесте, который целится в один R.
    #[test]
    fn freshness_is_one_at_zero_age() {
        let w = RankWeights::default();
        assert!((freshness(&w, 0.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn freshness_decreases_with_age_and_never_reaches_zero() {
        let w = RankWeights::default();
        let ages_days = [0.0, 1.0, 30.0, 180.0, 365.0, 3650.0];
        let mut previous = f64::MAX;
        for age in ages_days {
            let r = freshness(&w, age);
            assert!(r > 0.0 && r <= 1.0, "R ∈ (0, 1]: возраст {age} дн, R={r}");
            assert!(
                r <= previous,
                "R обязан монотонно убывать: возраст {age} дн"
            );
            previous = r;
        }
    }

    // --- T: t_machine < t_file < t_task (FR-007b) ---------------------------
    #[test]
    fn machine_types_rank_below_file_below_task() {
        let w = RankWeights::default();
        assert!(w.t_machine < w.t_file);
        assert!(w.t_file < w.t_task);
        for t in [
            w.t_decision,
            w.t_solution,
            w.t_concept,
            w.t_problem,
            w.t_task,
            w.t_file,
            w.t_machine,
            w.t_other,
        ] {
            assert!(t > 0.0 && t <= 1.0, "T ∈ (0, 1]: {t}");
        }
        assert!(w.t_other <= w.t_concept, "t_other не выше t_concept");
    }

    // --- A: A ∈ [1, 1.15], единственный множитель больше единицы -----------
    //
    // Через приватный `access_multiplier()` по той же причине, что и R выше:
    // P/R/T не должны участвовать в тесте, целящемся в A.
    #[test]
    fn access_multiplier_is_bounded() {
        let w = RankWeights::default();
        let lo = access_multiplier(&w, 0);
        let mid = access_multiplier(&w, 10);
        let hi = access_multiplier(&w, 1000);
        assert!((lo - 1.0).abs() < 1e-9, "A = 1 при access_count = 0");
        assert!((mid - 1.075).abs() < 1e-9, "A = 1 + 0.15 * 10/20 = 1.075");
        assert!(
            (hi - 1.15).abs() < 1e-9,
            "A потолок 1 + access_gain при access_count ≥ cap"
        );
        assert!(hi <= 1.0 + w.access_gain + 1e-9);
        assert!(
            lo >= 1.0,
            "A — единственный множитель, который может превысить 1"
        );
    }

    // --- арифметика FR-007b: годовое решение обгоняет вчерашний work_log ---
    #[test]
    fn fr_007b_a_year_old_decision_outranks_a_fresh_work_log() {
        let w = RankWeights::default();
        let now = fixed_now();
        // P = 0.7 у обоих: поля `confidence` нет — случай 76% базы.
        let decision = node(NodeType::Decision, json!({}), now - Duration::days(365), 0);
        let work_log = node(NodeType::WorkLog, json!({}), now - Duration::days(1), 0);

        let s_decision = score(&w, &decision, 1.0, now);
        let s_work_log = score(&w, &work_log, 1.0, now);

        assert!((s_decision - 0.231).abs() < 0.001, "{s_decision}");
        assert!((s_work_log - 0.104).abs() < 0.001, "{s_work_log}");
        assert!(
            s_decision > s_work_log,
            "решение обязано обгонять вчерашний work_log при равном провенансе"
        );
    }

    // --- r: медианный посев и узел-обходчик получают ровно 0.5 -------------
    #[test]
    fn normalize_bm25_gives_the_median_seed_exactly_half() {
        let raw = [10.0, 3.0, 7.0, 1.0, 5.0];
        let normalized = normalize_bm25(&raw);
        // Медиана |raw| — 5.0, лежит на индексе 4 исходного массива.
        assert!((normalized[4] - 0.5).abs() < 1e-9, "{:?}", normalized);
        for v in &normalized {
            assert!(*v >= 0.0 && *v < 1.0, "r ∈ [0, 1): {v}");
        }
    }

    #[test]
    fn normalize_bm25_empty_input_gives_empty_output() {
        assert!(normalize_bm25(&[]).is_empty());
    }

    // --- normalize_minmax: спред на весь [0, 1] даже на узком RRF-диапазоне -
    #[test]
    fn normalize_minmax_stretches_a_narrow_range_to_the_full_scale() {
        // Ровно то узкое соотношение, которое RRF(k=60) даёт рангам 1..12:
        // ~18% разброса между лучшим и худшим значением.
        let raw: Vec<f64> = (1..=12)
            .map(|rank: i32| 1.0 / (60.0 + rank as f64))
            .collect();
        let normalized = normalize_minmax(&raw);
        assert!(
            (normalized[0] - 1.0).abs() < 1e-9,
            "лучший (ранг 1) обязан получить 1: {normalized:?}"
        );
        assert!(
            normalized[11].abs() < 1e-9,
            "худший (ранг 12) обязан получить 0: {normalized:?}"
        );
        // Медианная нормировка на этом же входе даёт полосу 0,49…0,51
        // (измерено на живой базе 16.09.2026) — не тест на конкретное число
        // `normalize_bm25`, а контраст: мин-макс обязана давать заметно
        // больший спред на том же узком входе.
        let spread = normalized[0] - normalized[11];
        assert!(
            spread > 0.9,
            "мин-макс обязана растягивать на весь диапазон: {spread}"
        );
    }

    #[test]
    fn normalize_minmax_single_distinct_value_gives_the_midpoint() {
        assert_eq!(normalize_minmax(&[0.5, 0.5, 0.5]), vec![0.5, 0.5, 0.5]);
    }

    #[test]
    fn normalize_minmax_empty_input_gives_empty_output() {
        assert!(normalize_minmax(&[]).is_empty());
    }

    // --- RRF: слияние по рангам, не по релевантности ------------------------
    #[test]
    fn rrf_score_sums_both_lists_when_present_in_both() {
        // Первое место в обоих списках — наибольший возможный скор.
        let both_first = rrf_score(Some(1), Some(1), false);
        let one_only = rrf_score(Some(1), None, false);
        assert!((both_first - 2.0 / (RRF_K + 1.0)).abs() < 1e-9);
        assert!(
            both_first > one_only,
            "узел, найденный обоими движками, обязан обгонять найденный одним"
        );
    }

    #[test]
    fn rrf_score_never_penalizes_absence_from_the_other_list() {
        // Найден только dense-движком на первом месте — тот же вклад, что и
        // у чисто FTS-найденного на первом месте: складываются ранги, а не
        // штрафуется отсутствие во втором списке.
        let fts_only = rrf_score(Some(1), None, false);
        let dense_only = rrf_score(None, Some(1), false);
        assert!((fts_only - dense_only).abs() < 1e-12);
    }

    #[test]
    fn rrf_score_is_zero_when_absent_from_both() {
        assert_eq!(rrf_score(None, None, false), 0.0);
        assert_eq!(rrf_score(None, None, true), 0.0);
    }

    #[test]
    fn rrf_dense_first_beats_a_deep_hit_in_both_lists() {
        // f12702e1: при k = 60 узел на 50-м месте в обоих списках обгонял
        // dense-первое место. Правило слияния обязано держать обратное.
        assert!(rrf_score(None, Some(1), false) > rrf_score(Some(30), Some(30), false));
    }

    #[test]
    fn rrf_script_blind_node_gets_a_share_instead_of_zero() {
        let seen_nowhere = rrf_score(None, Some(3), false);
        let blind = rrf_score(None, Some(3), true);
        let expected = (1.0 + SCRIPT_BLIND_SHARE) / (RRF_K + 3.0);
        assert!((blind - expected).abs() < 1e-12);
        assert!(blind > seen_nowhere);
        // Нашла FTS — её ранг, а не замена.
        assert_eq!(
            rrf_score(Some(7), Some(3), true),
            rrf_score(Some(7), Some(3), false)
        );
    }
}
