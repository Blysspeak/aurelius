# Data Model: Память, которая приходит сама

**Feature**: 010-waking-memory | **Phase**: 1 | **Date**: 2026-09-08
**Вход**: [spec.md](./spec.md) (FR-001…FR-027), [plan.md](./plan.md) (стартовые параметры, фазы A…G)

**Владение, одной фразой**: этот документ владеет ТИПАМИ и ИМЕНАМИ ПОЛЕЙ, `contracts/mcp.md` —
ФОРМОЙ ПРОВОДА, и каждый его пример обязан быть ровно сериализацией объявленных здесь типов (те
же ключи, та же вложенность, та же необязательность), а формат файла кейсов
`fixtures/eval/cases.jsonl` не принадлежит ни одному из них — им владеет
`contracts/eval-cases.md`, и раздел 4 ниже его зеркало, а не второй источник. **Текстом совета в
отказе** не владеет ни один из троих — им владеет `contracts/cli.md` §4, и все остальные его
цитируют.

**Допущение о рабочем дереве, без которого часть ссылок ниже не проверяется.** Проверено
`git grep` по `HEAD`: `graph/pickup.rs`, `by_degree_then_recency`, `subgraph_degree` и флаг
`--prefix` команды `au recall` в коммите `HEAD` **не существуют** — они живут только в
незакоммиченном рабочем дереве этой ветки. Тот, кто возьмёт ветку из git, их не найдёт; всё, что
этот документ про них утверждает, верно для рабочего дерева на 2026-09-08.

Четыре сущности, ни одна из них не узел графа и ни одна не получает таблицу. Три живут ровно
столько, сколько длится вызов; четвёртая читается из файла и не пишется обратно.

| Сущность | Модуль | Строится в | Потребляется в | Жизнь |
|---|---|---|---|---|
| `RankWeights` | `graph/rank.rs` (**НОВЫЙ**) | `RankWeights::default()` на входе в recall | `graph/search.rs`, обработчик `memory_recall` (`handlers/session.rs`), `recognize.rs`, `eval.rs` | константа процесса, в БД не попадает |
| `WorkingState` | `graph/snapshot.rs` + `trace.rs` | `snapshot::working_state()` внутри `build_snapshot`/`snapshot_facts` | markdown-секция снимка во **всех** формах `au snapshot` и поле `state` в `au snapshot --json` | в памяти на один вызов; сериализуется в JSON, не хранится |
| `Recognition` | `graph/recognize.rs` (**НОВЫЙ**) | `recognize::scan()` из `au/src/hooks.rs` | `additionalContext` хука `UserPromptSubmit`; машинная форма — через `RecognitionReport` (раздел 3) | в памяти на один ход; сам тип не `Serialize`, наружу уходит проза или отдельный отчёт |
| `EvalCase` | `eval.rs` (**НОВЫЙ**) | `serde_json::from_str` по строкам `fixtures/eval/cases.jsonl` **после** отдельно разобранной первой строки `meta` | `eval::run()` | десериализуется на прогон, обратно не пишется |

`graph/render.rs` (**НОВЫЙ**) собственной сущности не заводит: это функция
`Node → String`, общая для recall и подачи. Она названа здесь потому, что три из четырёх
сущностей ссылаются на неё как на единственный способ превратить узел в строку.

Пятой строкой в таблице **не** становится `RecognitionReport` — сериализуемая машинная форма
узнавания (раздел 3): это не отдельная сущность, а проекция `Recognition` на провод, и владеет
её полями `contracts/cli.md` §1, а не этот документ.

---

## Грунтовка: что уже есть и с чем эти типы обязаны сойтись

Проверено чтением кода, а не памятью. Названия полей — дословные.

### `Node` (`crates/aurelius-core/src/models.rs`)

```rust
pub struct Node {
    pub id: Uuid,
    pub node_type: NodeType,
    pub label: String,
    pub note: Option<String>,
    pub source: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub memory_kind: MemoryKind,          // Semantic | Episodic
    pub last_accessed_at: DateTime<Utc>,
    pub access_count: i64,
    pub content_hash: Option<String>,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub deleted_at: Option<DateTime<Utc>>, // None = живой
    pub sync_seq: Option<i64>,
}
```

Полей `project`, `confidence`, `evidence`, `subject`, `measured_at` у `Node` **нет**. Всё это
лежит в `data` и достаётся `provenance::Provenance::from_data(&node.data)`:

```rust
pub struct Provenance {
    pub claim: Option<String>,
    pub evidence: Option<String>,
    pub measured_at: Option<DateTime<Utc>>,
    pub confidence: Option<Confidence>,   // Measured | Inferred | Reported | Unverified
    pub volatility: Option<Volatility>,
    pub verify_with: Option<String>,
    pub subject: Option<String>,
}
```

`confidence` — именно `Option`: «поля нет» и «явно записано `unverified`» различимы на этом
уровне и склеиваются только в `confidence_or_default()` (`provenance.rs:311`, возвращает
`Unverified` при `None`). Стартовая таблица плана требует для них разных множителей (0.7 против
0.6), поэтому `rank.rs` обязан читать `p.confidence` — `Option<Confidence>` — напрямую и **не**
звать `confidence_or_default()`: одна эта функция схлопывает 76 % базы (узлы без поля) в самый
низкий множитель. Проверено: `confidence_or_default()` зовут шестнадцать мест
(`graph/snapshot.rs:260`, `pickup.rs:342`, `handlers/*`, `commands.rs:319`) — ни одно из них не ранг,
и трогать их эта спека не просит.

Принадлежность проекту — не поле, а SQL-предикат `project_scope_sql` (`graph/search.rs:307`): метка
вида `[project] …` **или** ребро к узлу проекта. Всё, что скоупится по проекту, обязано ходить
через `typed_in_project` (`graph/search.rs:331`) / `get_tasks_filtered` (`graph/search.rs:410`), а не через
префикс метки.

`NodeType` выводит только `Debug, Clone, Serialize, Deserialize` (`models.rs:5`) — **ни
`PartialEq`, ни `Eq`, ни `Hash`**. Ключом `HashMap` он быть не может: `HashMap<NodeType, f64>`
не компилируется. Таблица множителей типа — функция `match`, а не карта. Узлы прогонов заводятся
как `NodeType::Custom("run".to_owned())` (`graph/mod.rs:53`, `link_evidence_run`) — проверено
чтением, а не по имени, — поэтому ветка `Custom(s)` в этом `match` обязательна, иначе `run` из
FR-007b получит нейтральный вес вместо машинного.

### `act_trace` (`db.rs:830`, миграция v9; API — `trace.rs`)

```sql
CREATE TABLE act_trace (
    id              INTEGER PRIMARY KEY,
    ts              INTEGER NOT NULL,   -- unix-секунды, Utc::now().timestamp()
    session_id      TEXT NOT NULL,
    kind            TEXT NOT NULL,      -- tool_call|file_edit|error|commit|msg_sent|user_correction
    payload         TEXT NOT NULL,      -- обрезан до 2000 символов (PAYLOAD_CAP)
    exit_code       INTEGER,
    state_hash_pre  TEXT,
    state_hash_post TEXT
);
CREATE INDEX idx_act_trace_session ON act_trace(session_id, ts);
```

Таблица append-only на триггерах (`act_trace_ro`, `act_trace_nodel`). Колонки `project` в ней
**нет** — это сказано и в доккомментарии `trace::files_edited_since`: «`act_trace` — одна таблица
на все проекты». Единственный существующий способ отделить свой проект от чужого —
префикс пути в `payload` строк `file_edit`, через `normalize_for_compare`/`normalized_prefix`
(снимают `\\?\`, приводят разделители и регистр). Для строк `tool_call` привязки к проекту нет
вовсе — см. ОВ-2.

### `SnapshotFacts` / `Fact` (`graph/snapshot.rs:128-156`)

```rust
pub struct Fact {
    pub kind: &'static str,   // userfact|active_task|task|problem|obligation|session|decision|concept|skill|digest
    pub text: String,
    pub at: Option<String>,   // RFC 3339 от updated_at; None у обязательств
    pub confidence: &'static str,
    pub stale: Option<String>,
}
pub struct SnapshotFacts { pub project: Option<String>, pub facts: Vec<Fact> }
```

### FTS (`db.rs:1021`, миграция v4; `SCHEMA_VERSION = 14`, `db.rs:8`)

```sql
CREATE VIRTUAL TABLE nodes_fts USING fts5(label, note, content='nodes', content_rowid='rowid');
```

Внешнее содержимое и ровно две колонки: `data` выкинута в v4 («raw JSON creates search noise»).
Отсюда два следствия, которые эта спека обязана держать в голове.

Первое: bm25 **не видит `claim`** — он лежит в `data`, а `data` из индекса выброшена. Значит
нормированный `r` в ранге считается по `label` и `note`, и никакая настройка весов этого не
меняет.

Второе, и это решение, а не наблюдение: **стемминг не получает колонки внутри `nodes_fts`**.
Измерено в `research.md` B.1 (`sqlite3 3.53.4`, одноразовая база, 2026-09-08): FTS5 не сверяет
колонки с контент-таблицей на DDL, поэтому `CREATE … fts5(label, note, label_stem,
content='nodes', …)` проходит, — но после него `INSERT INTO nodes_fts(nodes_fts)
VALUES('rebuild')` и любое чтение колонки у `nodes_fts` навсегда падают с
`no such column: T.label_stem`, потому что FTS5 тянет из контент-таблицы **весь** объявленный
набор колонок. Колонка-двойник в самой `nodes` это чинит, но это `ALTER TABLE nodes` — миграция
схемы узлов, вынесенная спекой в Out of Scope дословно.

Выбран третий вариант, `research.md` B.4: **отдельная автономная таблица**

```sql
CREATE VIRTUAL TABLE nodes_stem_fts USING fts5(label, note);  -- свой контент, без content=
```

`rowid` совпадает с `nodes.rowid`; `nodes_fts` не меняется ни на строку DDL. Это же даёт FR-021
даром — «нашлось точно» и «нашлось по стему» становятся двумя разными множествами, а не двумя
слагаемыми внутри одного числа bm25, — и FR-022 даром: пока таблица пуста, поиск идёт ровно как
сегодня. Ни один тип в этом документе стеммированной колонки внутри `nodes_fts` больше не
предполагает.

---

## 1. `RankWeights` — множители честного ранга

**Назначение.** Единственное место, где живут все числа формулы `score = r · P · R · A · T`
(FR-007), чтобы `au eval` калибровал их правкой одной структуры, а не охотой за константами
по трём крейтам (Constitution III: «не россыпь констант по месту»).

**Множителей ровно пять и шестого не будет.** Степень узла в подграфе (`subgraph_degree`,
`graph/mod.rs:145`) в произведение **не входит** — и это не значит, что она «работает в другом
месте»: **степень узла не участвует ни в видимости, ни в порядке: видимость задают рёбра и
потолки обхода, а `subgraph_degree` считается уже после обхода и кормит только сегодняшний
порядок, который эта спека убирает** (FR-007a). Проверено чтением, а не памятью: `walk`
(`crates/aurelius-core/src/graph/traverse.rs:105-163`) степень не считает и не читает — состав
решают рёбра и два потолка (`MAX_TRAVERSAL_NODES`, `MAX_TRAVERSAL_DEPTH`), — а посевы из
`search()` кладутся в результат безусловно, так что узел нулевой степени показывается. Это
проверяемое утверждение, а не оговорка: в `RankWeights` нет поля связности, в `score()` нет
параметра связности, и тест на это **односторонний** — пять равных множителей при разной степени
обязаны дать равный `score`. Прежняя формулировка требовала доказать вдобавок, что степень меняет
состав выдачи; это неверно, и такого теста не написать.

### Тип

```rust
// crates/aurelius-core/src/graph/rank.rs — НОВЫЙ модуль

#[derive(Debug, Clone, Copy)]
pub struct RankWeights {
    // --- P: провенанс, читается из Option<Confidence> --------------------
    pub p_measured: f64,    // 1.0
    pub p_reported: f64,    // 0.8
    pub p_inferred: f64,    // 0.75
    pub p_absent: f64,      // 0.7  — поля `confidence` в data НЕТ
    pub p_unverified: f64,  // 0.6  — поле ЕСТЬ и равно "unverified"

    // --- R: свежесть, R = 1 / (1 + age_days / freshness_scale_days) ------
    pub freshness_scale_days: f64,  // 180.0 — было 90.0, см. арифметику ниже

    // --- A: обращения, A = 1 + access_gain * min(ac, access_cap)/access_cap
    pub access_gain: f64,   // 0.15
    pub access_cap: i64,    // 20

    // --- T: множители типа узла ------------------------------------------
    pub t_decision: f64,    // 1.0
    pub t_solution: f64,    // 1.0
    pub t_concept: f64,     // 0.95
    pub t_problem: f64,     // 0.9
    pub t_task: f64,        // 0.85
    pub t_file: f64,        // 0.5
    pub t_machine: f64,     // 0.15 — Session, WorkLog, Custom("run"); было 0.3
    pub t_other: f64,       // 0.7  — нейтральный: тип, которого нет в списке выше

    // --- r: узлы, которых FTS не оценивал ---------------------------------
    pub r_traversed: f64,   // 0.5  — узел пришёл обходом, а не посевом

    // --- ворота, а не множители -------------------------------------------
    pub df_veto_ratio: f64,           // 0.01 — терм более чем в 1 % узлов
    pub min_significant_terms: usize, // 2
    pub fact_floor: f64,              // 0.5  — порог истории 3b
    pub fact_lead_ratio: f64,         // 1.5  — top1 >= 1.5 * top2
}

impl Default for RankWeights { /* стартовые догадки; калибруются `au eval` */ }
```

**Почему T — плоские поля, а не `HashMap<NodeType, f64>` и не вложенная `TypeWeights`.**
`NodeType` не выводит ни `PartialEq`, ни `Eq`, ни `Hash` (`models.rs:5`) — ключом карты он быть
не может, это не стилистика, а ошибка компиляции. Отображение типа в множитель — функция:

```rust
/// Единственное место, где тип узла превращается в число. Ни одна ветка не
/// падает в `unreachable!`: тип, которого нет в списке FR-007b, получает
/// нейтральный `t_other`, а не вес соседа по алфавиту.
pub fn type_weight(w: &RankWeights, t: &NodeType) -> f64 {
    match t {
        NodeType::Decision => w.t_decision,
        NodeType::Solution => w.t_solution,
        NodeType::Concept  => w.t_concept,
        NodeType::Problem  => w.t_problem,
        NodeType::Task     => w.t_task,
        NodeType::File     => w.t_file,
        NodeType::Session | NodeType::WorkLog => w.t_machine,
        NodeType::Custom(s) if s == "run"     => w.t_machine,
        _ => w.t_other,
    }
}
```

Ветка `Custom(s) if s == "run"` обязательна: узлы прогонов заводятся именно так
(`graph/mod.rs:53`, `link_evidence_run`), и без неё `run` из FR-007b уехал бы в `t_other = 0.7`,
то есть выше `file`. Все остальные штатные типы — `project`, `person`, `dependency`, `server`,
`module`, `crate`, `config`, `language`, `skill`, `user_fact`, `digest`, `doc` — и любой чужой
`Custom` получают `t_other = 0.7`. Значение выбрано нейтральным намеренно: `1.0` дал бы
импортированному чужому типу обгон над `concept`, а `0.9` молча топил бы каждый новый штатный
тип до тех пор, пока про него не вспомнят. `0.7` совпадает с `p_absent` не случайно — это одна
и та же мысль: «про этот узел ничего не сказано» не должно ни награждать, ни наказывать.

### Арифметика двух исправленных чисел

Стартовые `freshness_scale_days = 90` и `t_machine = 0.3` арифметически противоречили FR-007b:
при равном провенансе (случай «поля `confidence` нет» — 76 % базы, `P = 0.7`) годовое решение
проигрывало вчерашнему машинному `work_log`.

| Пара | `R` | `T` | `score` при `P = 0.7`, `A = 1` |
|---|---|---|---|
| **было**, τ = 90: решение возрастом 365 дней | `1/(1+365/90) = 0.198` | 1.0 | **0.139** |
| **было**, τ = 90: `work_log` возрастом 1 день | `1/(1+1/90) = 0.989` | 0.3 | **0.208** ← машина выигрывает в 1,5 раза |
| **стало**, τ = 180: решение возрастом 365 дней | `1/(1+365/180) = 0.330` | 1.0 | **0.231** ← решение выигрывает в 2,2 раза |
| **стало**, τ = 180: `work_log` возрастом 1 день | `1/(1+1/180) = 0.994` | 0.15 | **0.104** |

Оба новых числа — **по-прежнему догадки**, а не измерения: они выведены из одной пары узлов, и
единственное, что про них доказано, — что старая пара давала знак наоборот. Калибруются `au eval`
(фаза F), опровергаются замерами из `research.md` C (доля кейсов, где ожидаемый узел старше
180 дней и не попал в топ-5; распределение `access_count` по фикстуре).

### Сигнатуры

```rust
/// Нормированный bm25 внутри одной выдачи: r = |rank| / (|rank| + median|rank|).
/// Вход — только посевы FTS; узлы, пришедшие обходом, сюда не подаются вовсе.
pub fn normalize_bm25(raw: &[f64]) -> Vec<f64>;

/// `now` — параметр, а не `Utc::now()` внутри. `Utc::now()` на пути eval и
/// внутри ранжирующей функции запрещён: R зависит от даты прогона, и без
/// внешнего момента один и тот же кейс на замороженной фикстуре даёт разные
/// числа через неделю — прямо против FR-025. Текущий момент входит явным
/// параметром; в eval он берётся из `meta.as_of` файла кейсов, флаг `--now`
/// его перекрывает.
pub fn score(w: &RankWeights, node: &Node, r: f64, now: DateTime<Utc>) -> f64;
```

### Что получает `r` у узла, которого FTS не видел

Ответ, а не формула: **фиксированное нейтральное значение `r_traversed = 0.5`**.

Почему такой вопрос вообще есть. `context_with_report_seeded` (`traverse.rs:66`) берёт посевом
`RECALL_SEEDS = 12` узлов из `search()` (`traverse.rs:73`), а дальше BFS набирает соседей до
`MAX_TRAVERSAL_NODES = 200` (`traverse.rs:19`, глубина зажата `MAX_TRAVERSAL_DEPTH = 3`,
`traverse.rs:25`). У 188 узлов из 200 значения bm25 не существует — их никто не искал, до них
дошли по ребру. Вдобавок `search_ranked` (`graph/search.rs:102`) сегодня `rank` **не выбирает**: он
стоит только в `ORDER BY rank - (n.access_count * 0.1)` (`graph/search.rs:131`), в SELECT-лист не
входит, а после выборки порядок ещё раз переписывает `rank_by_matched_terms` (`graph/search.rs:142`)
по числу совпавших слов. Значит фаза B обязана добавить возврат bm25 из посевного запроса —
иначе нормировать нечего вовсе.

Почему 0.5, а не что-то другое:

- Нормировка `r = |rank| / (|rank| + median|rank|)` по построению даёт медианному посеву ровно
  `0.5`. Узел, пришедший обходом, ставится **на медиану посева**: он не выигрывает и не
  проигрывает у среднего найденного текстом, а его место решают оставшиеся четыре множителя —
  провенанс, свежесть, обращения, тип. Ровно этого и хочет FR-007a: связь приводит узел в
  выдачу, а порядок определяет содержательный вес.
- `r = 0` отвергнут: он обнулил бы `score` у 188 узлов из 200, и весь ранг выродился бы в
  перестановку двенадцати посевов. Это не ранжирование, а тот же посев с другим порядком.
- «Взять `r` соседа по ребру» отвергнут: тип связи в этом графе не несёт смысла релевантности
  (`Relation` — `relates_to`, `solves`, `belongs_to`…), и узел получал бы число, которого он не
  заработал, унаследованное через ребро, которое ничего про тему не утверждает.
- Отдельный множитель «пришёл обходом» отвергнут как шестой сигнал: пятью множителями и так
  не измерено ни одно значение, шестой добавлять до фазы F нечем.

`r_traversed` лежит в `RankWeights` именно потому, что это догадка: если на фикстуре окажется,
что состав топ-5 не меняется при `r_traversed ∈ [0.3, 0.7]`, нормировка bm25 не решает ничего и
её надо снимать целиком, а не подкручивать.

### Что тюнится, а что константа

**Тюнится** (поля `RankWeights`, `Default` несёт стартовые догадки, `au eval` двигает):
все пять множителей провенанса, `freshness_scale_days`, `access_gain`/`access_cap`, все восемь
множителей типа `t_*`, `r_traversed`, `df_veto_ratio`, `min_significant_terms`, `fact_floor`,
`fact_lead_ratio`.

**Константы компиляции**, которые eval двигать не имеет права, потому что их назначила спека или
они уже стоят в коде:

| Константа | Значение | Где | Почему не вес |
|---|---|---|---|
| бюджет узнавания | 50 токенов | FR-016 | назначен спекой |
| бюджет фактов 3b | 200 токенов | FR-016 | назначен спекой |
| бюджет блока состояния | 300 токенов | FR-001 | назначен спекой |
| потолок хука хода | 200 мс | FR-017 | назначен спекой |
| `RECALL_LIMIT` / `RECALL_TAIL_LIMIT` | 12 / 2 | `handlers/session.rs:14,18` | существует, не трогаем |
| `MAX_TRAVERSAL_NODES` / `MAX_TRAVERSAL_DEPTH` | 200 / 3 | `traverse.rs:19,25` | существует, спека 009 |
| `RECALL_SEEDS` | 12 | `traverse.rs:63` | существует |
| `W_LABEL`/`W_CLAIM`/`W_NOTE` | 3 / 2 / 1 | `graph/search.rs:160-162` | веса полей — другой механизм, до ранга |
| `OVERFETCH` | 5 | `graph/search.rs:84` | сколько выбрать до отсева, а не как ранжировать |

### Инварианты

- **Аргументов у `score()` ровно пять и связности среди них нет.** Инвариант проверяется тестом
  на подмену, и тест односторонний: два узла с одинаковыми `P`, `R`, `A`, `T`, `r` и разной
  степенью в подграфе обязаны дать **равный** `score`. Обратного утверждения тест не несёт:
  степень не решает и того, кто доедет до выдачи, — состав задают рёбра и два потолка обхода
  (FR-007a).
- `p_* ∈ (0, 1]`, порядок обязан быть невозрастающим:
  `p_measured ≥ p_reported ≥ p_inferred ≥ p_absent ≥ p_unverified`. Проверяется тестом, а не
  комментарием: FR-007 требует «промежуточных значений», а не произвольных.
  Внимание: этот порядок ставит `reported` **выше** `inferred`, тогда как объявление
  `enum Confidence` (`provenance.rs:37-47`) идёт `Measured, Inferred, Reported, Unverified`.
  Порядок объявления вариантов перечисления — не ранжирование, и источником весов служить не
  может; порядок плана взят как есть и никем не измерен.
- `p_absent` и `p_unverified` обязаны различаться, и различить их можно только читая
  `Option<Confidence>`. Тест: узел без ключа `confidence` в `data` и узел с
  `"confidence":"unverified"` при прочих равных дают разный `score`. Он же ловит случайный
  возврат к `confidence_or_default()`.
- `freshness_scale_days > 0`; `R ∈ (0, 1]`, `R = 1` при `age_days = 0`, монотонно убывает,
  нуля не достигает — FR-013: затухание не скрывает и не удаляет.
- `A ∈ [1, 1 + access_gain]` — **единственный множитель, который больше единицы**. Отсюда
  `score ∈ [0, 1 + access_gain)`, при стартовых числах — `[0, 1.15)`. Порог `fact_floor = 0.5`
  читается в этой шкале, не в шкале `[0,1]`.
- `T ∈ (0, 1]`, `t_machine < t_file < t_task` (FR-007b), и `t_other` не выше `t_concept`.
- `r ∈ [0, 1)` по построению нормировки: медианный **посев** всегда получает ровно `0.5`, и
  ровно столько же получает узел, пришедший обходом (`r_traversed`). Величина относительная —
  сравнима внутри одной выдачи и **не сравнима между выдачами**.
- `df_veto_ratio ∈ (0, 1)`; один и тот же порог обслуживает FR-006 (отказ recall) и FR-015
  (терм не считается сущностью). Две копии числа разошлись бы молча, а спека называет обе
  ситуации одним «N %».

### Где строится и кем потребляется

- Строится: `RankWeights::default()` в `graph::search` (recall CLI/`au context`),
  в `handlers::session::memory_recall`, в `recognize::scan`, в `eval::run`.
- Потребляется **на одном пути — recall, — и общей функцией служит сам `rank::score`, а не новый
  компаратор**. Сегодня порядок recall задан замыканием `handlers/session.rs:253-259`, и это
  дословная копия уже существующей `graph::by_degree_then_recency` (`graph/mod.rs:157-165`) —
  сравнено построчно: тот же `degree.get().copied().unwrap_or(0)`, тот же
  `db.cmp(&da).then(b.created_at.cmp(&a.created_at))`. Работа состоит из одного шага и ни одного
  нового модуля-экстракта: `handlers/session.rs` удаляет своё замыкание вместе с подсчётом степени
  (`handlers/session.rs:222-226`) и сортирует `rank::score`.
- **`by_degree_then_recency` при этом не трогается.** Её зовут два места `au pickup`
  (`pickup.rs:335,370`), где порядок по степени в подграфе — заявленное назначение команды, а не
  наследство. `au pickup` в этом цикле отдаёт то же, что и сегодня; его перевод — отдельная
  задача **вне** этой фичи. Третьего компаратора не заводится: дубль исчезает потому, что одна из
  двух копий удаляется, а не потому, что обе перешли на новое.
- `subgraph_degree` (`graph/mod.rs:145`) остаётся в коде ради `pickup`, но в видимости обхода не
  участвует и в `score()` не входит (см. «Множителей ровно пять» выше).

### Жизнь

Значение в стеке на время вызова. Ни в базу, ни в конфиг, ни в env — новых хранилищ спека не
заводит (Out of Scope). `au eval` при развёртке подаёт нестандартную структуру аргументом.

---

## 2. `WorkingState` — «где остановились»

**Назначение.** Вычисляемый срез по проекту текущего каталога, из которого рендерится первая
секция снимка и поле `state` машинной формы; ничего не хранит и ничего не пишет (FR-002).

Имена полей ниже нормативны. Форма провода — сериализация ровно этих полей в ровно этом
порядке — записана в `contracts/mcp.md` §3.2 и обязана совпадать с ними ключ в ключ.

### Тип

```rust
// crates/aurelius-core/src/graph/snapshot.rs

#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkingState {
    pub project: String,
    /// `git rev-parse --abbrev-ref HEAD` в каталоге проекта. `None` — не
    /// репозиторий, detached HEAD, git недоступен. Не ошибка (edge case спеки).
    pub branch: Option<String>,
    pub active_task: Option<StateTask>,
    /// Не более трёх (FR-001). Режется первым при нехватке бюджета.
    pub decisions: Vec<StateLine>,
    /// Режется вторым.
    pub blockers: Vec<StateLine>,
    pub last_turn: LastTurn,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StateTask {
    pub id8: String,          // первые 8 символов UUID, как в pickup.rs
    pub label: String,
    pub status: String,       // data.status
    pub activated_at: Option<DateTime<Utc>>, // tasks::TaskFields::activated_at
    /// true, когда последний ход шёл не по этой задаче (edge case спеки:
    /// «называет ход, а задачу помечает как активную, но не последнюю»).
    pub not_the_last_worked: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StateLine {
    pub id8: String,
    pub kind: &'static str,   // "decision" | "problem" | "task"
    /// Уже отрендеренная строка: `render::one_line(node)`, единственный способ
    /// превратить узел в текст в этой спеке.
    pub text: String,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LastTurn {
    /// Последняя группа строк `act_trace` с одним `session_id`.
    Traced {
        session_id: String,
        at: DateTime<Utc>,        // из ts последней строки группы
        age_days: i64,
        tool_calls: usize,
        files: Vec<String>,       // payload строк file_edit под корнем проекта
        last_error: Option<String>, // payload последней строки kind='error'
        /// Как доказана принадлежность проекту — см. ОВ-2.
        scope: TurnScope,
    },
    /// Ходы были, но ни одного tool-call: «последний ход» помечен неизвестным,
    /// а не выдуман (acceptance 5 истории 1). `at` — из последнего узла
    /// `Session` проекта, если он есть.
    Untraced { at: Option<DateTime<Utc>>, age_days: Option<i64> },
    /// По проекту нет ни одного хода. Блок целиком не рендерится (FR-005).
    None,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnScope {
    /// Хотя бы одна строка file_edit сессии лежит под корнем проекта.
    ProjectPath,
    /// Строки есть, но ни одна не несёт пути — привязать к проекту нечем.
    Unattributed,
}
```

### Инварианты и правила проверки

- `decisions.len() ≤ 3` (FR-001).
- Порядок усечения при нехватке бюджета жёсткий: сначала `decisions`, потом `blockers`,
  **никогда** `active_task` (FR-001).
- Бюджет считается в символах, как весь остальной снимок (`B_IDENTITY`…`B_DIGEST`,
  `clip()` по границе слова). Сами константы в сумме дают **4300** (600+1000+800+900+500+500,
  `graph/snapshot.rs:19-24`); доккомментарий над ними (`graph/snapshot.rs:18`) округляет сумму
  до «~4500» и называет соотношение «~4500 символов — порядка 1.5К токенов», то есть ~3 символа
  на токен; 300 токенов — это ≈900 символов. Соотношение взято из того комментария, **не
  измерено**; точная константа `B_STATE` фиксируется в фазе C замером на фикстуре. Там же, где
  считается зазор до потолка ulika (8000, `MAX_SNAPSHOT`), берётся 4300, а не пересказ:
  8000 − 4300 = 3700 символов — арифметика по константам, а не длина реального снимка.
- `is_empty()` истинно, когда `active_task.is_none() && decisions.is_empty() &&
  blockers.is_empty() && matches!(last_turn, LastTurn::None)`. Пустое не рендерится (FR-005)
  и не сериализуется.
- `age_days` объявляется в тексте, только если > 1 суток (FR-004).
- Ни одного `INSERT`/`UPDATE` на этом пути. `consolidate()`, который `commands::snapshot`
  дёргает раз в сутки (`commands.rs:2715-2729`), — существующая запись снимка и к этой
  сущности отношения не имеет; правило «только чтение» из FR-014a относится к хуку хода, не
  к снимку.
- Координаты секретов отсеиваются `secret::is_secret_ref` — тем же фильтром, что уже стоит в
  `layer()` и `push_facts()`.

### Где строится и кем потребляется

- Строится: `snapshot::working_state(conn, project, now) -> Result<WorkingState>`; читает
  `act_trace` через новую функцию в `trace.rs` («последний ход» — последняя группа записей
  одной сессии), активную задачу — `get_tasks_filtered(conn, Some(p), Some("active"), None, 1)`,
  решения — `typed_in_project(&NodeType::Decision, …)`, блокеры —
  `get_unsolved_problems` + задачи со статусом `blocked`, корень проекта —
  `tasks::project_root` (`data.path` узла проекта), ветку — `git -C <root> rev-parse
  --abbrev-ref HEAD` тем же приёмом, что `tasks::current_commit_sha` (готовой функции для
  ветки в репозитории нет — проверено).
- Потребляется: `build_snapshot` (`graph/snapshot.rs:299`) — первой секцией `## 1 · Где остановились`,
  номера остальных восьми секций сдвигаются (FR-002c; `parseMemory` в
  `ulika/hooks/lib/brief.mjs:108` номер отбрасывает регуляркой
  `/^##\s*(?:\d+\s*·\s*)?(.+?)\s*$/` — проверено). Заголовок не должен попадать под
  `SERVICE = /архив|дистиллят/i`, а строки — начинаться со слов
  `EMPTY_LINE = /^(хвостов нет|чисто|пусто|—)\b/i` (`ulika/hooks/lib/brief.mjs:86-87`), иначе ulika сочтёт
  секцию служебной или пустой.

**Часы — параметр, и это меняет сигнатуры до самого верха.** `age_days` последнего хода и
пометка давности в тексте блока считаются от момента, который приходит **снаружи**: иначе кейс
`snapshot_contains` с ожиданием «2 дн. назад» зеленеет ровно одни сутки (FR-025, запрет D-2
`contracts/eval-cases.md` §4). Утверждение «правок сигнатур не требуется» неверно; вот полный
список того, что меняется, — он объявлен работой, а не деталью:

- `snapshot::working_state(conn, project, now)` — новая функция, `now` в ней с рождения;
- `build_snapshot` (`graph/snapshot.rs:299`, сегодня `(conn, project)`) получает третьим аргументом
  `now: DateTime<Utc>`; из него же берётся `ts` шапки (`graph/snapshot.rs:300`, сегодня `Utc::now()`);
- `snapshot_facts` (`graph/snapshot.rs:268`, сегодня `(conn, project)`) — то же самое: `state` в
  машинной форме считается тем же моментом, что и markdown, иначе две формы разойдутся по
  давности на одном и том же вызове;
- боевые вызывающие: `crates/au/src/commands.rs:2734` (markdown) и `commands.rs:2731`
  (`--json`), `crates/aurelius/src/mcp/handlers/snapshot.rs:22` (`memory_snapshot`). Все трое
  подают `Utc::now()` — запрет на `Utc::now()` касается пути eval и функции ранга, а не
  боевого входа;
- **тесты, и их больше, чем боевых вызовов.** `build_snapshot` зовут десять тестов
  (`graph/snapshot.rs:476,511,550,570,681,719,776,800,886,940`), `snapshot_facts` — четыре
  (`graph/snapshot.rs:586,622,806,897`) плюс один в `graph/session.rs:394`. Пятнадцать вызовов
  правятся вместе с сигнатурой; момент в тестах фиксированный, и именно это делает проверяемым
  текст про давность.

**Путь доставки — не флаг `--hook`, и сборщиков двое.** Проверено в рабочем дереве, и это
исправляет прежнюю формулировку «блок собирается в `build_snapshot`, а значит приходит во все
формы»: `au snapshot --json` до `build_snapshot` не доходит вовсе. Ветка
`if json_out { … return … }` (`commands.rs:2731`) отдаёт `graph::snapshot_facts`
(`graph/snapshot.rs:268`) и возвращается раньше вызова `build_snapshot` (`commands.rs:2734`). Значит:

- `build_snapshot` закрывает **три markdown-формы**: `au snapshot --project <p>`,
  `au snapshot --hook` и MCP-инструмент `memory_snapshot` (`handlers/snapshot.rs:22`);
- поле `state` машинной формы вставляется **отдельно, в `snapshot_facts`**, из того же вызова
  `working_state`. Одно вычисление, две точки вставки; второй формулы состояния не заводится.

Флаг `--hook` путём доставки при этом не является ни для одной из форм:

- Детектор ulika — регулярка **по границе слова**, а не по строке с флагом:
  `AU_SNAPSHOT_RE = /(^|[\s/\\])au(\.exe)?\s+snapshot\b/` (`ulika/hooks/lib/restore.mjs:91`).
  Она ловит любой вызов `au snapshot`, с `--hook` и без.
- Записи `au snapshot --hook` в `plugin/hooks.json` рабочего дерева **нет** — она удалена
  незакоммиченной правкой этой ветки; в файле шесть хуков: `skills`, `db backup`
  (`SessionStart`), `touch`, `trace` (`PostToolUse`), `reindex`, `judge` (`Stop`). На машине при
  этом стоит установленный плагин 3.4.4, который эту строку ещё несёт, — то есть «зарегистрирован
  или нет» зависит от того, куда смотреть, и именно поэтому доставку нельзя привязывать к
  регистрации хука.
- Сам снимок ulika печатает обеими формами и обе — **без** `--hook`: `au snapshot --project <p>`
  (`ulika/hooks/lib/restore.mjs:185`) и `au snapshot --project <p> --json`
  (`ulika/hooks/lib/brief.mjs:191`). Вдобавок `--json` и `--hook` объявлены `conflicts_with`
  (`crates/au/src/main.rs:649-650`), то есть «блок в `--hook`» и «`state` в `--json`» физически
  не могут быть одним вызовом.

**Когда владелец это увидит.** Восстановление после разрыва контекста покрыто ровно двумя
случаями: старт сессии и `/clear`. Проверено: `RESTORE_ON = new Set(['clear', 'startup'])`
(`ulika/hooks/lib/restore.mjs:58`), и `compact` с `resume` исключены там намеренно — у компакции
есть собственная сводка той же сессии, а возобновление держит контекст в окне. Значит блок
«где остановились» после `/compact` и после `resume` **не показывается**, и это вне области
данной фичи, а не её недоработка.

- `snapshot_facts` (`graph/snapshot.rs:268`) — новым полем верхнего уровня:

```rust
pub struct SnapshotFacts {
    pub project: Option<String>,
    pub facts: Vec<Fact>,                        // не меняется
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<WorkingState>,             // новое, ПОСЛЕДНИМ полем
}
```

Поле обязано быть последним и с `skip_serializing_if`: тест
`json_facts_shape_is_fixed_and_empty_means_nothing_to_say` (`graph/snapshot.rs:581-594`) сравнивает
сериализацию **побайтово** — `assert_eq!(serde_json::to_string(&out), r#"{"project":"пусто",
"facts":[]}"#)`. При пустом состоянии строка остаётся той же и тест остаётся зелёным без правки.

### Отношение к уже существующему `au pickup`

`graph/pickup.rs` (`build_pickup`, `pickup.rs:415`; структуры `Pickup:57`, `Tail:66`,
`FacetRow:77`, `Facets:85`, `Record:91`, `CriticalTask:102`, `Critical:110`) отвечает на тот же
вопрос — «что было до разрыва контекста»: якорный фасет, хвост последней сессии с `next_steps`,
открытые записи, критические задачи, пять бюджетов в символах, ранжирование по степени в
подграфе через `by_degree_then_recency` (`pickup.rs:335,370`).

`WorkingState` строится **не рядом с ним, а поверх него**, и это условие приёмки фазы C, а не
пожелание:

- Всё, что `pickup` уже умеет доставать (активная задача, открытые проблемы, критические
  задачи), берётся его функциями. Второй запрос за тем же — это второй ответ на тот же вопрос,
  и расходиться они начнут в тот день, когда кто-то поправит один.
- Собственная часть `WorkingState` — ровно то, чего у `pickup` нет: последний ход из `act_trace`
  и ветка git.
- **Порядок `au pickup` в этом цикле не меняется.** `by_degree_then_recency` остаётся за ним
  (`pickup.rs:335,370`), на `rank::score` переходит только путь recall; перевод `pickup` —
  отдельная задача вне этой фичи (раздел 1). Следствие названо прямо: пока задача не сделана,
  `au pickup` и recall упорядочивают одни и те же узлы по-разному, и это решение, а не забытый
  хвост. `WorkingState` берёт у `pickup` состав (что доставать), а не порядок.

Что при этом остаётся различным по назначению: `au pickup` — команда по требованию с пятью
бюджетами, `WorkingState` — секция снимка с одним бюджетом ≤ 300 токенов. Сводить их в одну
структуру спека не просит; она просит, чтобы они не отвечали разное на один вопрос.

### Жизнь

В памяти на один вызов снимка. Ничего не пишется, `Stop` не трогается, `SessionEnd` не нужен
(clarification спеки). В JSON уходит только как срез момента.

---

## 3. `Recognition` — что узнано в сообщении

**Назначение.** Результат сопоставления входящего сообщения со словарём лейблов проекта с
учётом df: какая сущность, насколько она частотна, где именно в тексте, и прошла ли ворота.

### Тип

```rust
// crates/aurelius-core/src/graph/recognize.rs — НОВЫЙ модуль

/// Пустой результат — нормальный и ожидаемый исход большинства ходов
/// (clarification спеки, SC-005). Поэтому `Default`, а не `Option<Recognition>`:
/// «ничего не узнано» — это значение, а не отсутствие ответа.
#[derive(Debug, Clone, Default)]
pub struct Recognition {
    /// Пусто — подача ноль байт, код 0 (FR-017).
    pub entities: Vec<RecognizedEntity>,
    /// Сколько токенов сообщения просмотрено. Меньше общего числа — сообщение
    /// обрезано по потолку (вставленный лог, edge case спеки).
    pub tokens_scanned: usize,
    pub tokens_total: usize,
    /// Сколько узнанных отброшено df-вето. Не для вывода — для `au eval`.
    pub vetoed: usize,
    pub elapsed: std::time::Duration,
}

#[derive(Debug, Clone)]
pub struct RecognizedEntity {
    pub node_id: Uuid,
    pub label: String,
    pub node_type: NodeType,
    pub df: DocFrequency,
    pub span: TokenSpan,
    /// Давность работы: `max(updated_at)` по узлам, связанным с сущностью.
    /// `None` — сущность есть, работы по ней нет.
    pub last_worked_at: Option<DateTime<Utc>>,
    /// Открытые вопросы: задачи в статусах active|blocked|backlog плюс
    /// проблемы без ребра `solves` (`get_unsolved_problems`).
    pub open_questions: usize,
    /// Факты истории 3b. Пусто, пока ворота `fact_floor`/`fact_lead_ratio`
    /// не пройдены, и пусто всегда до фазы G.
    pub facts: Vec<StateLine>,
}

/// Документная частота терма по живым узлам проекта.
#[derive(Debug, Clone, Copy)]
pub struct DocFrequency { pub hits: i64, pub total: i64 }

impl DocFrequency {
    /// `total == 0` → 0.0, а не деление на ноль: пустая база — не «терм
    /// встречается везде».
    pub fn ratio(self) -> f64;
}

/// Границы совпадения в ИСХОДНОМ сообщении, в БАЙТАХ.
#[derive(Debug, Clone, Copy)]
pub struct TokenSpan { pub start: usize, pub end: usize }
```

### Инварианты и правила проверки

- **Пустой случай представим и является нормой.** `Recognition::default()` → подача ноль
  байт и код возврата 0 (FR-017). Пустая подача выдаётся и тогда, когда:
  сущностей не узнано; все узнанные срезаны df-вето; `elapsed > 200 мс`; каталог не
  принадлежит проекту (`find_project_by_label` по имени каталога вернул `None`).
- **Ворота df**: сущностью считается только `df.ratio() <= weights.df_veto_ratio`
  (FR-015). При стартовом 1 % и словаре ~12 800 лейблов на 16 333 живых узлах терм `au`
  с df = 3,5 % вето проходит и сущностью не становится — это и есть третий Independent Test
  истории 3.
- **Границы токена, а не подстроки** (edge case спеки: путь вида
  `crates/au/src/commands.rs`, кавычки; отдельного файла `judge.rs` в дереве нет — `au judge`
  это функция `judge_cmd`, `crates/au/src/commands.rs:2950`).
  Предикат левой границы уже написан — `probes.rs:88`, `starts_at_boundary(hay, start)`:
  символ слева не должен быть буквой, цифрой или одним из `_ - . / \ @ ~`. Он **приватный**
  (`fn`, не `pub fn`) и проверяет только левый край. Для `recognize.rs` его надо поднять до
  `pub(crate)` и добавить симметричный правый — копировать нельзя, две копии разойдутся.
- `span` — байтовые смещения. Хранить символьные нельзя: `starts_at_boundary` режет
  `hay[..start]` байтами. В этом же коде уже ловили обратную ошибку — `window_around`
  (`handlers/mod.rs`) специально пересчитывает байтовое смещение `find` в символьное перед
  резкой кириллицы.
- `entities` отсортированы по `last_worked_at` убыванием; в подачу идут первые три, остальные
  называются числом (acceptance 4 истории 3).
- **Ни одной записи в базу** (FR-014a): ни `touch_node` (`graph/crud.rs:392`), ни
  `trace::ingest`, ни `add_node`. Отдельно про `touch_node`: он не трогает `updated_at`, но его
  `UPDATE nodes` взводит триггер `nodes_au`, а тот переписывает строку FTS (`delete` +
  `insert`). Одна «безобидная» отметка доступа — три записи в WAL.
- **Соединение открывается только на чтение, и `db::open` для этого не годится.** Проверено:
  `db::open` (`db.rs:96`) зовёт `ensure_wal` (`db.rs:132` — `PRAGMA journal_mode=WAL`, а перевод
  журнала берёт исключительную блокировку файла) и затем `migrate`. Хук хода, который на каждом
  сообщении берёт блокировку и готов мигрировать схему, — это не «только чтение», это запись,
  отложенная до первой неудачной гонки. В `db.rs:154` уже есть
  `fn open_readonly(path) -> Result<Connection>` — `Connection::open_with_flags(path,
  SQLITE_OPEN_READ_ONLY)` плюс `busy_timeout`, без WAL и без миграций. Функция **приватная**, и
  поднимают её до `pub` **в фазе A, а не в фазе D**: первый её потребитель — `au eval`, который
  обязан открыть фикстуру только на чтение на самом первом прогоне, то есть до того, как
  появится хук хода. Фаза D эту функцию уже застаёт публичной и просто зовёт её из
  `recognize_hook`. Побочный эффект в подарок: на базе, чья схема новее бинарника, хук просто
  ничего не найдёт вместо того, чтобы упасть на проверке версии.
- Координаты секретов не попадают в подачу (FR-018): узлы отсеиваются `secret::is_secret_ref`
  ещё при сборке словаря, а собранная строка перед выводом проходит
  `secret::scan_text_for_lookalike`.

### Где строится и кем потребляется

- Строится: `recognize::scan(conn, project, message, &weights, now) -> Result<Recognition>`,
  вызывается из `au/src/hooks.rs::recognize_hook`, куда JSON события `UserPromptSubmit`
  приходит на stdin (`prompt` / `user_prompt`, `cwd`, `session_id` — так их читает и
  `route.mjs`).
- Потребляется: `hooks.rs` рендерит прозой и печатает
  `{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"…"}}` — форма
  проверена по `ulika/hooks/route.mjs:88-96`. Оба хука висят на одном событии, их
  `additionalContext` склеиваются (FR-014b), поэтому 50 токенов — потолок только своей части.
- Регистрируется в `plugin/hooks.json` (FR-019) — и это **единственная** правка этого файла во
  всей фиче, она принадлежит фазе D. Фаза C (блок «где остановились») `plugin/hooks.json` не
  трогает вовсе: блок едет через `build_snapshot` и `snapshot_facts`, а не через новый хук.
  Правка манифеста обязана идти **одной задачей** с правкой теста
  `crates/au/tests/plugin_manifest.rs:157`, где список подкоманд захардкожен как
  `["db", "judge", "reindex", "skills", "touch", "trace"]` и сверяется через `assert_eq!` с
  сообщением «must be exactly … (six total)». Добавить `recognize` в манифест и не тронуть
  строку 157 — значит покрасить `cargo test --workspace` в красный и оставить его красным на
  фазы E, F и G, где причина покраснения уже не будет очевидна.

### Машинная форма: `RecognitionReport`

Запрет «в JSON не сериализуется даже в отладке» снят, потому что он противоречил контракту:
`contracts/cli.md` §1 требует от `au recognize <TEXT> --json` объект с `project`, `entities[]`,
`skipped_by_df`, `elapsed_ms` и `context`, а `elapsed_ms` — то самое число, по которому ворота
фазы D считают p95 для SC-004. Форма, которой нет, не измеряется.

Разведено на два типа, а не на один с `#[derive(Serialize)]`:

- **`Recognition` остаётся внутренним.** В нём `Uuid`, `NodeType`, `TokenSpan` в **байтовых**
  смещениях и `Duration` — внутренности расчёта. Сериализовать их значит объявить их контрактом
  провода и потом не иметь права поменять байты на символы.
- **`RecognitionReport` (`graph/recognize.rs`, `serde::Serialize`)** — машинная форма. Строится
  из `Recognition` и момента входа в `main` (отсюда `elapsed_ms`, которого у `Recognition` нет:
  у него есть только своя `elapsed`). Имена и состав полей нормативны в `contracts/cli.md` §1 —
  здесь тип назван, но не описан вторично: два описания одной формы и есть та ошибка, ради
  которой в шапке записано владение.

### Жизнь

В памяти на один ход. Сам `Recognition` наружу не уходит ни в какой форме: в хуке печатается
проза, в ручной форме — `RecognitionReport`. Ни следа в базе — по этому и проверяется SC-004.

---

## 4. `EvalCase` — одна строка `fixtures/eval/cases.jsonl`

**Назначение.** Один кейс: вход плюс ожидание, ровно в одном из пяти видов проверки.

**Нормативный источник формата — `contracts/eval-cases.md`.** Этот раздел — его зеркало в
терминах типов Rust, а не второе описание. Разошлись — прав `contracts/eval-cases.md`, и
расхождение чинится правкой этого раздела.

### Файл: первая строка — не кейс

```
строка 1  {"meta": {"version": 1, "as_of": "<RFC3339>", "fixture": "<путь к .db от корня репо>", "fixture_sha256": "<hex>"}}
строка 2… {"id": "<стабильный слаг>", "kind": "<один из пяти>", "input": {…}, "expect": {…}, "why": "<кто решил и когда>", "tags": ["by:owner"]}
```

Это и есть та деталь, из-за которой прежнее описание загрузчика не работало: «построчный
`serde_json::from_str::<EvalCase>`» на первой строке падает — у неё нет ни `id`, ни `kind` из
словаря проверок. Загрузчик обязан разбирать первую строку отдельным типом.

`meta.as_of` — **часы прогона**. Возраст узлов, а значит множитель свежести `R`, считается
относительно него, а не относительно «сегодня». `Utc::now()` запрещён и на пути eval, и внутри
ранжирующей функции: без внешнего момента один и тот же кейс на замороженной фикстуре даёт
разные числа через неделю (FR-025). Флаг `au eval --now <RFC3339>` перекрывает `meta.as_of` —
он нужен ровно затем, чтобы проверить чувствительность к дате, не трогая файл кейсов.

`as_of` живёт в одном месте, а не в каждом кейсе: пятьдесят пять копий одной даты — это
пятьдесят пять шансов разойтись. `fixture_sha256` считается по **распакованному** `.db`: на
порядок выдачи влияет содержимое базы, а не байты архива.

### Тип

```rust
// crates/aurelius-core/src/eval.rs — НОВЫЙ модуль

/// Первая строка файла. Отдельный тип, а не вариант `EvalCase`: у неё нет ни
/// `id`, ни `kind`, и общий разбор построчно на ней падает.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct MetaLine {
    pub meta: EvalMeta,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct EvalMeta {
    /// Версия формата этого контракта. Сейчас 1. Незнакомая версия — отказ
    /// прогона, а не попытка разобрать «как получится».
    pub version: u32,
    /// Часы прогона: момент, относительно которого считается возраст узлов.
    pub as_of: DateTime<Utc>,
    /// Путь к распакованной базе относительно корня репозитория.
    pub fixture: String,
    /// sha256 распакованного `.db`, hex. Сверяется ДО прогона.
    pub fixture_sha256: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct EvalCase {
    /// Стабильный слаг, kebab-case, ASCII. Уникален в файле; освободившийся
    /// после удаления кейса `id` мёртв навсегда — на него ссылаются отчёты.
    pub id: String,
    /// Одна строка прозы: кто назвал этот ответ верным и когда (§1.4
    /// eval-cases.md). Не объект: правило «measured без команды не
    /// принимается» держится ревью, а не типом.
    pub why: String,
    /// Метки кейса. Ровно одна из пространства `by:` обязательна
    /// (`by:owner` | `by:derived` | `by:agent:<метка>`) — по ней отчёт строит
    /// разбивку авторства (FR-027); остальные метки группируют и на вердикт
    /// не влияют. Пусто или без метки `by:*` — сломанный файл.
    pub tags: Vec<String>,
    /// Дискриминатор — `kind`. Внутренне тегированное перечисление: строка
    /// файла остаётся плоским объектом с ключами `kind`/`input`/`expect`.
    #[serde(flatten)]
    pub body: CaseBody,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaseBody {
    /// Нужный узел входит в топ-5 recall по теме (SC-002, SC-002a, FR-007…009).
    RecallTop5 { input: RecallInput, expect: RecallExpect },
    /// Запрос в косвенном падеже даёт непустую выдачу (SC-003, FR-020, FR-023).
    Morphology { input: MorphologyInput, expect: MorphologyExpect },
    /// Слишком широкая тема получает отказ, а не ответ (SC-001, FR-006).
    Refusal { input: RefusalInput, expect: RefusalExpect },
    /// Снимок содержит ожидаемое, блок стоит первым и влезает в бюджет
    /// (FR-001…FR-005, SC-006). Пол по числу кейсов не назначен; вид
    /// закрывает ворота фазы C.
    SnapshotContains { input: SnapshotInput, expect: SnapshotExpect },
    /// Подача узнавания называет что надо и молчит где надо. Пол не назначен;
    /// вид закрывает ворота ДВУХ фаз: половина `empty: true` — механическая
    /// проверка FR-017 и SC-005, она закрывает фазу D; половина с фактами
    /// закрывает фазу G.
    RecognizeFeed { input: RecognizeInput, expect: RecognizeExpect },
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RecallInput {
    pub topic: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub depth: Option<u32>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RecallExpect {
    /// UUID узлов фикстуры.
    pub top5: Vec<Uuid>,
    /// `any` (умолчание) — хотя бы один из `top5` на позициях 1…5; `all` — все.
    #[serde(default)]
    pub mode: Option<TopMode>,
    /// SC-002a: ни одного узла этих типов на позициях 1…5.
    #[serde(default)]
    pub forbid_types: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct MorphologyInput { pub query: String }

#[derive(Debug, Clone, serde::Deserialize)]
pub struct MorphologyExpect {
    pub non_empty: bool,
    /// Если задан — этот UUID присутствует на любой позиции, не только в топ-5.
    #[serde(default)]
    pub contains: Option<Uuid>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RefusalInput { pub topic: String }

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RefusalExpect {
    pub refused: bool,
    /// Ожидаемый повод отказа: `topic_too_broad` | `topic_too_narrow`.
    /// Задан — обязан совпасть. Именно он ловит подмену повода: на теме «au»
    /// истинны оба условия, и правильный ответ — первый.
    #[serde(default)]
    pub reason: Option<String>,
    /// Порог снизу, а не равенство: фаза E меняет индекс и поднимет df почти
    /// всех тем.
    #[serde(default)]
    pub matched_at_least: Option<i64>,
    /// Подстрока совета. Самим текстом совета владеет `contracts/cli.md` §4 —
    /// и этот документ, и `contracts/eval-cases.md` его только цитируют.
    /// Нормативный кейс `refuse-topic-au` держится за подстроку «не сужает».
    #[serde(default)]
    pub advice_contains: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SnapshotInput {
    pub project: String,
    /// `markdown` (умолчание) — то, что отдаёт `build_snapshot`; `json` —
    /// поле `state` машинной формы, которое собирает `snapshot_facts`.
    #[serde(default)]
    pub form: Option<SnapshotForm>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotForm { Markdown, Json }

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SnapshotExpect {
    /// `true` — блок «где остановились» стоит ПЕРВОЙ секцией; `false` — блока
    /// в снимке нет вовсе (проект без трассы, FR-005).
    #[serde(default)]
    pub first_section: Option<bool>,
    /// Все подстроки обязаны найтись, побайтово.
    #[serde(default)]
    pub contains: Vec<String>,
    /// Ни одной из этих подстрок нет. Здесь живут UUID и наносекунды.
    #[serde(default)]
    pub absent: Vec<String>,
    /// Длина блока в символах не больше указанной — прокси бюджета SC-006.
    #[serde(default)]
    pub max_chars: Option<usize>,
    /// Только при `form: json`: каждый ключ есть в объекте `state` и не `null`.
    #[serde(default)]
    pub state_keys: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RecognizeInput {
    /// Ключ называется `prompt`, как поле события `UserPromptSubmit`, а не
    /// `message`: кейс подаёт ровно то, что придёт хуку.
    pub prompt: String,
    #[serde(default)]
    pub project: Option<String>,
    /// Каталог, по которому определяется проект, когда `project` не задан.
    /// Им же пишется кейс «каталог вне графа → подача пуста».
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RecognizeExpect {
    /// Истина — подача обязана быть пустой (ноль байт). Взаимоисключимо с
    /// непустым `names`.
    pub empty: bool,
    /// Имена сущностей, обязанные прозвучать в подаче.
    #[serde(default)]
    pub names: Vec<String>,
    /// `any` (умолчание) — хотя бы одно имя из `names`; `all` — все.
    #[serde(default)]
    pub names_mode: Option<TopMode>,
    /// Ни одно из этих имён прозвучать не имеет права. Здесь живёт FR-015:
    /// имя, отсеянное по df, названо быть не может.
    #[serde(default)]
    pub forbid_names: Vec<String>,
    /// Прокси бюджета FR-016: 50 токенов ≈ 150 символов, коэффициент **не
    /// измерен**.
    #[serde(default)]
    pub max_chars: Option<usize>,
}
```

`TopMode` (`any` | `all`) объявлен в `contracts/eval-cases.md` §2.1 и здесь только назван.
Никакого словаря `by`/`confidence` в типах кейса нет: авторство несёт одна строковая метка в
`tags`, а `why` — проза.

### Загрузка и прогон

```rust
/// Первая строка — `MetaLine`, остальные — `EvalCase`. Битая строка называет
/// СВОЙ номер в файле (нумерация с 1, вместе с meta) и не роняет прогон целиком.
pub fn load(path: &Path) -> Result<(EvalMeta, Vec<EvalCase>)>;

/// `now` подаётся снаружи: `meta.as_of`, либо флаг `--now`. Внутри — ни одного
/// `Utc::now()`.
pub fn run(
    conn: &Connection,
    meta: &EvalMeta,
    cases: &[EvalCase],
    w: &RankWeights,
    now: DateTime<Utc>,
) -> EvalReport;
```

- Пустой файл — ошибка «нет строки `meta`», а не «ноль кейсов»: файл без часов прогона
  невоспроизводим по построению.
- Файл из одной строки `meta` — «нет кейсов» по каждому виду, код 0, не паника.
- `version`, которой этот бинарник не знает, — отказ прогона с названием версии.
- `fixture_sha256` не сошёлся — отказ прогона. Число «после», снятое на другой базе, несравнимо
  с числом «до», и молча сравнить их хуже, чем не сравнить вовсе.

### Пороги по видам (FR-027 в редакции ревизии)

| `kind` | Пол | Откуда |
|---|---|---|
| `recall_top5` | ≥ 30 | SC-002 («на 30 кейсах») |
| `morphology` | ≥ 20, из них ≥ 3 отрицательных (`non_empty: false`) | SC-003 («20 запросов») |
| `refusal` | ≥ 5, из них ≥ 2 отрицательных (`refused: false`) | SC-001 плюс защита от переотказа |
| `snapshot_contains` | пола нет в этом цикле | ворота фазы C |
| `recognize_feed` | пола нет в этом цикле | ворота фаз D (пустота) и G (факты) |

Итого **не менее 55 строк** кейсов. Прежняя формулировка «не менее 30» была неоднозначна ровно
там, где это дороже всего: тридцать кейсов одного вида и тридцать, размазанных по пяти видам, —
это разные наборы, и второй не измеряет ни одного из трёх обещанных чисел.

**Оба пола наполняются в фазе A, и после этого набор заморожен.** Тридцать `recall_top5` и
двадцать `morphology` пишутся до первого замера, потому что базовая линия «до» снимается на них
же: число, снятое на десяти кейсах, и число, снятое на тридцати, — не сравнимы, и разница между
ними прочиталась бы как результат работы. Фаза F кейсов этих двух видов **не добавляет**: она
добавляет кейсы отказа и пересчитывает «после» на замороженном наборе.

### Отчёт (FR-024 в редакции ревизии)

Отчёт печатает **долю пройденных по каждому из пяти видов**, а не по трём. Три вида с полом
закрываются фазой F; `snapshot_contains` — частью ворот фазы C; `recognize_feed` разрезан между
двумя фазами: кейсы `empty: true` — механическая проверка FR-017 и SC-005 — входят в ворота
**фазы D**, кейсы с фактами закрывают **фазу G**. Ворота фазы D поэтому обязаны перечислять
кейсы этого вида: без них «подача молчит, когда узнавать нечего» остаётся утверждением, а не
проверкой. Именно это делает поле `state` снимка измеримым механически, а не глазами: доля
`snapshot_contains` и есть его потребитель.

Вердикт кейса — `PASS`, `FAIL` или `SKIP`; `SKIP` не входит ни в числитель, ни в знаменатель.
Сломанный кейс (UUID нет в фикстуре, `expect` не разбирается) — это `SKIP` с причиной, а не
`FAIL`: иначе протухание фикстуры читается как регрессия поиска, а спор «стало лучше или хуже»
решается ровно этим отчётом. По проваленному печатается ожидаемое и полученное (FR-026), а не
«FAIL» — для `recall_top5` это фактическая позиция узла или «не найден» (разница между
«ранжирование промахнулось» и «поиск не нашёл» — это разница между фазой B и фазой E).

### Инварианты и правила проверки

- `id` уникален в файле; дубль — сломанный файл, а не проваленный кейс.
- В `tags` ровно одна метка `by:*`; её отсутствие — сломанный файл, а не `SKIP`: без авторства
  разбивка отчёта (FR-027) перестаёт что-либо значить.
- `expect.refused = true` осмысленно только само по себе: отказ означает, что узлы не отдаются
  вовсе. То же для `RecognizeExpect { empty: true, names: [...] }` — взаимоисключимо.
- Прогон детерминирован (FR-025): порядок кейсов — порядок строк файла, никакой параллельности,
  `now` подаётся снаружи, фикстура открывается **только на чтение** (`db::open_readonly`,
  `db.rs:154`, поднимается до `pub` в фазе A — её первый потребитель и есть `au eval`, см.
  раздел 3) и после прогона байт в байт та же.
- **На пути `au eval` никто не зовёт `touch_node`.** `access_count` инкрементирует только
  обработчик MCP `memory_recall` (`handlers/session.rs:266`); общая функция ранга и
  `au recall --topic` этого не делают. Инвариант не косметический: соединение к фикстуре
  read-only, и один спрятанный инкремент превратил бы каждый кейс `recall_top5` в отказ записи —
  то есть в провал прогона, а не кейса.
- Обезличивание: `prompt`, `contains` и `advice_contains` снимаются с живой базы, поэтому перед
  фиксацией прогоняются через `secret::scan_text_for_lookalike` (`crates/aurelius-core/src/secret.rs:248`) — та же
  заслонка, что стоит на снимке и поиске.

### Где строится и кем потребляется

- Строится: `eval::load(path)`.
- Потребляется: `eval::run(...)`. Проверки зовут **боевые** функции — `build_snapshot`
  (а при `form: json` — `snapshot_facts`), тот же путь recall, `recognize::scan` — а не свои
  копии (Structure Decision плана).
- CLI: `au eval <файл кейсов>`, флаги `--now` и «живая база» (последний помечает отчёт
  «несравнимо») — `contracts/cli.md`.

### Жизнь

Читается на прогон, обратно не пишется. Фикстура распаковывается из
`fixtures/eval/aurelius-2026-09-08.db.zst` и открывается только на чтение.

---

## Что НЕ меняется

**Таблицы узлов и рёбер.** Ни `nodes`, ни `edges` не получают колонок и не теряют их. Всё, что
эта спека кладёт в узел, кладётся туда, куда уже кладут: в `data`, через `Provenance`. Список
колонок в SELECT (`NODE_COLS`, `graph/search.rs:288`) и `row_to_node`/`row_to_edge` (`graph/mod.rs`)
остаются как есть. `SCHEMA_VERSION = 14` (`db.rs:8`) не двигается фазами A–D; поднимает его
только фаза E, и только ради новой автономной таблицы `nodes_stem_fts` — сама `nodes_fts`
(`db.rs:1021`) не меняется ни на строку DDL.

**Контракт `au snapshot --json`.** Проверено по коду, который его печатает
(`commands.rs:2711-2735` → `graph::snapshot_facts` → `serde_json::to_string(&SnapshotFacts)`), и
по коду, который его читает (`ulika/hooks/lib/au.mjs`, `latestSession`).

- `facts` остаётся массивом; порядок наполнения в `snapshot_facts` (`userfact`, `active_task`,
  `task`, `problem`, `obligation`, `session`, `decision`, `concept`, `skill`, `digest`) не
  трогается.
- `facts[].kind` — прежний замкнутый список `&'static str`.
- `facts[].text` — `claim` → `note` → `label`, без бюджетной обрезки и без подмешанных пометок.
- `facts[].at` — `updated_at.to_rfc3339()`, **формат не меняется**. Это не украшение:
  `latestSession` (`ulika/hooks/lib/au.mjs:206-216`) сравнивает `at` строкой, потому что идентификатора узла в
  снимке нет, и «наш это узел или чужой» решается точным совпадением метки времени. Там же
  срезаются наносекунды регуляркой `/(\.\d{3})\d+/` — то есть длина дробной части тоже часть
  фактического контракта.
- **`Fact` несёт пять полей, а не три.** Проверено чтением `graph/snapshot.rs:128-145`: `kind`,
  `text`, `at`, `confidence` (`&'static str`, из `confidence_or_default()`, `graph/snapshot.rs:260`) и
  `stale` (`Option<String>`, приписка «старше N дней — перепроверь …»). FR-002b называет только
  первые три; `confidence` и `stale` существуют в коде и точно так же не трогаются — они
  перечислены здесь именно затем, чтобы их отсутствие в спеке не прочли как разрешение их
  переставить или выбросить.
- Сторож этого контракта — не обзор, а один `assert_eq!`. Тест
  `json_facts_shape_is_fixed_and_empty_means_nothing_to_say` (`graph/snapshot.rs:581-594`) сравнивает
  **всю строку** `serde_json::to_string(&SnapshotFacts)` побайтово с
  `{"project":"пусто","facts":[]}`. Любое новое поле, сериализуемое безусловно, роняет его —
  включая безобидное на вид `"state":null`.
- Новое поле `state` — верхнего уровня, не элемент `facts` (FR-002b), **последним**, с
  `#[serde(skip_serializing_if = "Option::is_none")]`. Так строка при пустом состоянии не
  меняется и тест остаётся зелёным без правки: контракт, который он стережёт, не ослабляется.

**Форма markdown-снимка.** Заголовки `## N · Название`, слова-маркеры пустоты (`EMPTY_DIGEST =
"Хвостов нет — чисто."`), резка по границе слова `clip()`, бюджеты слоёв — как есть. Меняется
только нумерация: новая секция становится первой.

**Существующее ранжирование в других местах.** `subgraph_degree` (`graph/mod.rs:145`) остаётся
и продолжает считать степень — но не «для обхода»: она считается уже **после** обхода, по рёбрам
готового результата, и кормит порядок `au pickup`. `by_degree_then_recency` (`graph/mod.rs:157`)
тоже остаётся нетронутой: её зовут два места `pickup.rs:335,370`, и в этом цикле `au pickup`
отдаёт то же, что и сегодня. `matched_score` (`graph/search.rs:166`) с весами полей
`W_LABEL/W_CLAIM/W_NOTE` остаётся ступенью до ранга; `SearchOutcome::diagnosis` (`graph/search.rs:31`)
и подсказка префикса остаются (FR-023).

Что **не** попадает в этот список: замыкание-компаратор в `handlers/session.rs:253-259` вместе с
подсчётом степени `handlers/session.rs:222-226`. Их удаляют, и путь recall сортирует `rank::score`.
Дубль компаратора после этого исчезает — не потому, что обе копии перешли на новое, а потому,
что одна из двух удалена. Перевод `au pickup` на общий ранг — отдельная задача вне этой фичи, и
до неё две команды сознательно упорядочивают узлы по-разному.

---


## Открытые вопросы

Найдены при сверке спеки и плана с кодом. Ревизия закрыла **семь из девяти** (ОВ-1, 3, 4, 5, 6,
7, 8); открытыми остались два — ОВ-2 и ОВ-9. Закрытые оставлены здесь с ответом, а не
вычеркнуты, — вычеркнутый вопрос выглядит как незаданный, и его задают второй раз.

### Закрыты ревизией

**ОВ-1. У большинства узлов выдачи нет bm25 вовсе — ЗАКРЫТ.** Наблюдение остаётся верным:
`context_with_report_seeded` (`traverse.rs:66`) берёт `RECALL_SEEDS = 12` посевов из FTS, а BFS
набирает до `MAX_TRAVERSAL_NODES = 200` соседей, которых FTS не видел; вдобавок `search_ranked`
сегодня `rank` из SQL **не выбирает** (`ORDER BY rank - (n.access_count * 0.1)`, `graph/search.rs:131`;
в SELECT-лист `rank` не входит). Ответ: посевной запрос начинает возвращать bm25, нормировка
считается по посевам, а узлы обхода получают фиксированное нейтральное `r_traversed = 0.5` —
ровно медиану посева. Развёрнуто в разделе 1, «Что получает `r` у узла, которого FTS не видел»,
вместе с тремя отвергнутыми вариантами.

**ОВ-3. Порядок `reported` и `inferred` — ЗАКРЫТ.** Оставлен порядок плана
(`measured 1.0 > reported 0.8 > inferred 0.75`), хотя объявление `enum Confidence`
(`provenance.rs:37-47`) идёт `Measured, Inferred, Reported, Unverified`. Основание: порядок
вариантов перечисления — это порядок объявления, а не ранжирование, и источником весов служить
не может. Оба числа остаются догадками и калибруются `au eval`.

**ОВ-4. Половина типов узлов не имела множителя T — ЗАКРЫТ.** `t_other = 0.7`, нейтральный;
`Custom("run")` уходит в `t_machine = 0.15` отдельной веткой `match`. Разбор в разделе 1.

**ОВ-5. Недетерминизм по времени — ЗАКРЫТ.** `now` — параметр `score()`; на пути eval он
приходит из `meta.as_of` первой строки файла кейсов, флаг `--now` его перекрывает; `Utc::now()`
на этом пути запрещён. Вторая половина вопроса — что порог `fact_floor = 0.5` абсолютный, а `r`
относительный — остаётся арифметикой, а не неизвестностью, и при `freshness_scale_days = 180`
читается так: при `P = T = 1`, `A ≤ 1.15` и `r < 1` условие `score ≥ 0.5` требует `R ≳ 0.435`,
то есть возраст не старше ~234 дней **в самом лучшем случае**; при правдоподобных `r ≈ 0.7` и
`A = 1` — не старше ~72 дней. SC-007 требует находить факт месячной давности, так что запас
есть, но он вычислен, а не измерен, и первый же прогон `au eval` его проверит.

**ОВ-6. Стеммированная колонка не помещается в нынешний FTS — ЗАКРЫТ.** Выбрана отдельная
автономная таблица `nodes_stem_fts` (`research.md` B.4). Ни колонки в `nodes_fts`, ни
`ALTER TABLE nodes`. Разбор — в «Грунтовке», раздел про FTS.

**ОВ-7. Доставка блока привязана к несуществующей регистрации хука — ЗАКРЫТ.** Блок виден во
всех формах `au snapshot`, включая `--json`, которым его и читает ulika, но сборщиков **двое**:
`build_snapshot` — три markdown-формы, `snapshot_facts` — поле `state` машинной формы. Флаг
`--hook` путём доставки не является ни для одной. Проверенные факты — в разделе 2, «Путь
доставки».

**ОВ-8. Пересечение с `au pickup` — ЗАКРЫТ.** `WorkingState` строится поверх `pickup.rs`, а не
рядом: у него берётся состав, а не порядок. Порядок `au pickup` в этом цикле не меняется —
`by_degree_then_recency` остаётся за ним, на `rank::score` переходит только путь recall, а
перевод `pickup` вынесен в отдельную задачу вне фичи. Разбор — раздел 2, «Отношение к уже
существующему `au pickup`».

### Остаются открытыми

**ОВ-2. Блок «где остановились» нельзя достоверно ограничить проектом.** FR-003 требует
ограничения проектом текущего каталога, а в `act_trace` колонки `project` нет (миграция v9,
`db.rs:830`). Единственный существующий фильтр — префикс пути в `payload` строк `file_edit`
(`trace::files_edited_since`, `trace.rs:121`, через `normalized_prefix`/`normalize_for_compare`,
`trace.rs:155,167`), и он не применим к строкам `tool_call`, которые несут команду, а не путь.
Ход, состоявший из одних `Bash`-вызовов в чужом проекте, попадёт в блок соседнего. В типе это
отражено полем `TurnScope`, но само правило — считать ход своим при нуле путей или не считать —
не назначено. Решается фазой C замером: сколько ходов на живой машине приходят
`Unattributed`.

**ОВ-9. SC-004 нечем измерить в лоб.** «Счётчик WAL-транзакций до и после» на живой машине
считает и чужие записи: `route.mjs` ulika висит на том же `UserPromptSubmit` и при пересечении
порога контекста пишет чекпоинт через `au`. Это законно (clarification: правило «только чтение»
на ulika не распространяется), но означает, что счётчик надо снимать по своему соединению, а не
по файлу базы. Открытым остаётся именно как: соединение хука read-only (`db::open_readonly`),
поэтому у него счётчика транзакций записи попросту нет — и доказательством, вероятно, станет сам
факт открытия с `SQLITE_OPEN_READ_ONLY`, а не измерение. Формулировка проверки — за фазой D.

---

## Правки после ревизии

Каждая строка — одно изменение и решение, которое оно закрывает. Ничего не переписано молча.

- **Владение документами названо явно** в шапке: типы и имена полей — здесь, форма провода —
  `contracts/mcp.md`, формат файла кейсов — `contracts/eval-cases.md`. Закрывает находку
  ревизии «два описания одного формата с разными именами ключей».
- **Раздел 4 переписан целиком под D1**: первая строка `meta` с `version`/`as_of`/`fixture`/
  `fixture_sha256`, дискриминатор `kind` (было `check`), пять значений вместо трёх,
  `input`/`expect`/`why` вместо плоских `expect_*`, отдельный тип `MetaLine` и загрузчик,
  который эту строку читает, а не давится ею.
- **Пороги по видам кейсов (D2)**: 30 `recall_top5` + 20 `morphology` + 5 `refusal` = не менее
  55; у `snapshot_contains` и `recognize_feed` пола в этом цикле нет. Прежнее «≥ 30 кейсов»
  удалено как неоднозначное.
- **Отчёт по пяти видам (D3)**, а не по трём; названо, какая фаза каким видом закрывается. Это
  и даёт полю `state` механического потребителя.
- **Стемминг переписан на отдельную таблицу `nodes_stem_fts` (D4)** в «Грунтовке»; упоминание
  стеммированной колонки внутри `nodes_fts` удалено вместе с ОВ-6.
- **`freshness_scale_days` 90 → 180 и `t_machine` 0.3 → 0.15 (D5)**, с таблицей арифметики: было
  0.139 против 0.208 (машина выигрывала в 1,5 раза), стало 0.231 против 0.104 (решение выигрывает
  в 2,2 раза). Оба числа помечены как догадки.
- **`TypeWeights` заменена плоскими `t_*` и функцией `type_weight()` с `match` (D6)**; добавлены
  `t_other = 0.7` для всех неназванных типов и ветка `Custom("run") → t_machine`. Причина —
  `NodeType` не выводит ни `Eq`, ни `Hash` (`models.rs:5`), проверено.
- **Провенанс читается как `Option<Confidence>` (D7)**; добавлен инвариант-тест «узел без поля и
  узел с `unverified` дают разный `score`», ловящий возврат к `confidence_or_default()`.
- **Связность объявлена не-сигналом (D8)**: в `RankWeights` нет поля связности, в `score()` нет
  параметра связности, добавлен инвариант «равные множители при разной степени дают равный
  `score`». Формулировка «степень — фильтр видимости» усилена до проверяемой.
- **Открытие базы хуком — `db::open_readonly` (D9)**, с разбором, что `db::open` (`db.rs:96`)
  зовёт `ensure_wal` (`db.rs:132`, исключительная блокировка) и `migrate`. Прежнее умолчание об
  этом убрано.
- **Порядок recall переводится через существующую `by_degree_then_recency` (D11)**, а не через
  новый экстракт; добавлен раздел «Отношение к уже существующему `au pickup`». Из «Что НЕ
  меняется» удалена строка, обещавшая `pickup` старый порядок, — она противоречила D11.
- **Доставка блока состояния отвязана от `--hook` (D12)**: названы регулярка
  `ulika/hooks/lib/restore.mjs:91` и удалённая из рабочего дерева запись `plugin/hooks.json` при живой 3.4.4 на
  машине.
- **Область восстановления сужена до `startup` и `clear` (D13)**, `compact` и `resume` названы
  вне области со ссылкой на `ulika/hooks/lib/restore.mjs:58`.
- **Правка `plugin/hooks.json` отнесена к фазе D и связана с тестом (D14)**
  `crates/au/tests/plugin_manifest.rs:157`; сказано прямо, что фаза C этот файл не трогает.
- **Исправлены ссылки file:line**, найденные ревизией неверными: `project_scope_sql`
  `graph/search.rs:292` → `graph/search.rs:307`; `MAX_TRAVERSAL_NODES`/`MAX_TRAVERSAL_DEPTH`
  `traverse.rs:20,26` → `traverse.rs:19,25`; `SnapshotFacts`/`Fact` `graph/snapshot.rs:126-155` →
  `graph/snapshot.rs:128-156`. Добавлены недостающие: `db.rs:8,96,132,154,1021`, `models.rs:5`,
  `graph/mod.rs:53,145,157`, `graph/search.rs:31,84,102,131,142,166,331,410`, `graph/crud.rs:392,481`,
  `pickup.rs:57…415`, `trace.rs:121,155,167`, `graph/snapshot.rs:268,299,581-594`, `crates/au/src/main.rs:649-650`.
- **Раздел «Что НЕ меняется» исправлен**: `Fact` — пять полей (проверено, `graph/snapshot.rs:128-145`),
  и сторожит их `assert_eq!` по всей строке сериализации, а не обзор.
- **Открытые вопросы разделены** на закрытые ревизией (ОВ-1, 3, 4, 5, 6, 7, 8 — с ответом) и
  оставшиеся открытыми (ОВ-2, ОВ-9). Ни один не удалён.
- **Названо расхождение, которое ревизия не сняла**: `why` в сокращённой записи D1 — строка, в
  нормативном `contracts/eval-cases.md` §1.4 — объект. Взят объект; причина записана в разделе 4.
  Если нормативный документ выберет строку, править придётся здесь.

### Вторая ревизия

- **C3. Раздел 4 переписан зеркалом замороженного формата.** `why` — **строка**, а не объект;
  добавлено обязательное поле `tags: Vec<String>` ровно с одной меткой `by:*`; у `refusal`
  появился `reason`; у `snapshot_contains` вход стал `{project, form}`, а ожидание —
  `first_section`/`contains`/`absent`/`max_chars`/`state_keys`; у `recognize_feed` вход стал
  `{prompt, project?, cwd?}` (ключ `prompt`, не `message`), ожидание —
  `empty`/`names`/`names_mode`/`forbid_names`/`max_chars`. Эта строка **отменяет** предыдущую
  строку списка про «взят объект `why`»: расхождения больше нет, и абзац, который его называл,
  удалён вместе со ссылкой на несуществующий словарь `by`/`confidence`.
- **C4. Запрет сериализовать узнавание снят.** `Recognition` остаётся внутренним, машинную форму
  несёт отдельный тип `RecognitionReport` (`graph/recognize.rs`, `Serialize`); состав его полей
  нормативен в `contracts/cli.md` §1. Прежнее «не сериализуется даже в отладке» прямо
  противоречило `au recognize --json`, чьё поле `elapsed_ms` и есть число ворот фазы D для
  SC-004.
- **C15. Часы протянуты параметром до самого верха, и это названо работой.** `working_state`
  рождается с `now`; `build_snapshot` (`graph/snapshot.rs:299`) и `snapshot_facts` (`graph/snapshot.rs:268`)
  получают третий аргумент; правятся боевые вызовы `commands.rs:2731,2734` и
  `handlers/snapshot.rs:22` и пятнадцать вызовов в тестах
  (`graph/snapshot.rs:476,511,550,570,586,622,681,719,776,800,806,886,897,940`, `graph/session.rs:394`).
  Утверждение «правок сигнатур не требуется» снято как неверное.
- **C1. Формулировка про степень узла исправлена по коду** в разделе 1 (дважды), в «Что НЕ
  меняется» и в инвариантах. `walk` (`traverse.rs:105-163`) степень не считает и не читает,
  посевы кладутся безусловно, `subgraph_degree` (`graph/mod.rs:145`) считается уже после обхода.
  Тест стал односторонним: пять равных множителей при разной степени — равный `score`.
- **C17. `au pickup` в этом цикле порядок не меняет.** `by_degree_then_recency` остаётся за ним
  (`pickup.rs:335,370`); путь recall удаляет своё замыкание (`handlers/session.rs:253-259`) вместе с
  подсчётом степени (`handlers/session.rs:222-226`) и сортирует `rank::score`; третьего компаратора не
  заводится. Эта строка **отменяет** строку про D11 выше и переписанный ею фрагмент «Что НЕ
  меняется», а также ответ ОВ-8.
- **C16. Доставка блока состояния разведена на двух сборщиков**: `build_snapshot` — три
  markdown-формы (`au snapshot --project`, `--hook`, `memory_snapshot`), `snapshot_facts` — поле
  `state` машинной формы; ветка `if json_out` (`commands.rs:2731`) до `build_snapshot`
  (`commands.rs:2734`) не доходит. Названы оба вызова ulika: `ulika/hooks/lib/restore.mjs:185` и `ulika/hooks/lib/brief.mjs:191`
  вместо прежнего `ulika/hooks/lib/restore.mjs:183-186`.
- **C7. `db::open_readonly` публикуется в фазе A, а не в фазе D** (разделы 3 и 4): первый
  потребитель — `au eval`, которому фикстура нужна read-only на самом первом прогоне.
- **C8. Полы 30 `recall_top5` и 20 `morphology` наполняются в фазе A**, набор после этого
  заморожен; фаза F добавляет только кейсы отказа и пересчитывает «после». Добавлены
  отрицательные подполы (≥ 3 морфологии, ≥ 2 отказа) — зеркало §1.5 нормативного документа.
- **C9. `recognize_feed` разрезан между фазами**: `empty: true` закрывает ворота фазы D
  (FR-017, SC-005), кейсы с фактами — ворота фазы G. Исправлено и в перечислении `CaseBody`, и в
  таблице полов, и в разделе про отчёт.
- **C10. Назван единственный, кто инкрементирует `access_count`** — обработчик MCP
  `memory_recall` (`handlers/session.rs:266`); на пути `au eval` и `au recall --topic` `touch_node` не
  зовётся, иначе read-only фикстура роняет прогон.
- **C18. Добавлено допущение о рабочем дереве** в шапку: `graph/pickup.rs`,
  `by_degree_then_recency`, `subgraph_degree` и `au recall --prefix` в `HEAD` отсутствуют
  (проверено `git grep` по `HEAD`).
- **C19. Ссылки:** `graph/mod.rs:150` → `graph/mod.rs:145` (три места в тексте и одно в списке
  правок первой ревизии); `provenance.rs:35-37` → `provenance.rs:37-47` (два места);
  `crates/au/src/main.rs:649` → `crates/au/src/main.rs:649-650`; несуществующий путь `crates/au/src/judge.rs` заменён на
  реальный — `au judge` это `judge_cmd`, `crates/au/src/commands.rs:2950`. Счёт закрытых
  открытых вопросов исправлен: **семь из девяти**, а не шесть.
- **Владение переформулировано одной фразой**: типы и имена полей — здесь, форма провода —
  `contracts/mcp.md`, формат файла кейсов — `contracts/eval-cases.md`.

### Третья ревизия

- **K3. Назван владелец текста совета — `contracts/cli.md` §4**: в шапке («Владение, одной
  фразой») и доккомментарием над `RefusalExpect::advice_contains` в разделе 4. Этот документ
  владеет **типом поля**, но не его содержимым; прежде указатель отсутствовал вовсе, и
  кольцевая ссылка «tasks → eval-cases → cli» замыкалась мимо него.
- **K2. Самого текста совета этот документ не несёт** — он объявляет только поле
  `advice_contains: Option<String>`, поэтому заменять было нечего; вместо текста добавлен
  указатель на владельца и напоминание про подстроку «не сужает», за которую держится
  нормативный кейс `refuse-topic-au`.
- **K5. `traverse.rs:104-152` → `traverse.rs:105-163`** (раздел 1 и строка C1 в списке правок
  второй ревизии): 104 — последняя строка доккомментария, тело `walk` идёт 105…163. Утверждение
  «`walk` степень не считает и не читает» перепроверено на исправленном диапазоне и осталось
  верным.
- **K5. `handlers/session.rs:221-225` → `:222-226`** (три места: `RankWeights` → «Где строится и
  кем потребляется», раздел «Что НЕ меняется» и строка C17 списка правок второй ревизии): 221 —
  хвост комментария, подсчёт степени занимает 222…226.
- **K4. Дублирующиеся базовые имена получили каталог.** В дереве по два файла с именами
  `session.rs`, `crud.rs`, `search.rs`, `snapshot.rs`, `secret.rs`, а `restore.mjs` лежит и в
  `ulika/hooks/`, и в `ulika/hooks/lib/`: голая ссылка разрешалась в обоих и означала разный
  код. Проставлены `handlers/session.rs`, `graph/snapshot.rs`, `graph/search.rs`,
  `graph/crud.rs`, `crates/aurelius-core/src/secret.rs`, `ulika/hooks/lib/brief.mjs`.
  `graph/session.rs:394` (пятнадцатый вызов `snapshot_facts`) каталог нёс и раньше — именно
  поэтому его и видно рядом с четырнадцатью вызовами из `graph/snapshot.rs`.
- **K4, сверх названного аудитом: `main.rs` в дереве трижды** (`crates/au/src/`,
  `crates/aurelius/src/`, `crates/aurelius-sync-server/src/`), и строка 649 есть только в
  первом — два других файла короче ста строк. Голые ссылки «main.rs:649-650» в списках правок
  заменены на `crates/au/src/main.rs:649-650`; в тексте про `conflicts_with` полный путь стоял и
  раньше.
- **K8. Сумма бюджетов слоёв — 4300, а не 4500** (инварианты `WorkingState`):
  600+1000+800+900+500+500 по `graph/snapshot.rs:19-24`. «~4500» — округление в доккомментарии
  `graph/snapshot.rs:18`; соотношение «~3 символа на токен» по-прежнему берётся оттуда и остаётся
  **не измеренным**. Зазор до потолка ulika пересчитан от 4300: 8000 − 4300 = 3700 символов, и
  это арифметика по константам, а не длина реального снимка.
- **K1 и K7 сверены, правок не потребовали**: формулировка про степень узла в разделе 1 уже
  дословно совпадает с канонической, а разрез `recognize_feed` (половина `empty: true` — ворота
  фазы D, половина с фактами — ворота фазы G) и `snapshot_contains` как ворота фазы C уже стоят
  одинаково в `CaseBody`, в таблице полов и в разделе про отчёт.
