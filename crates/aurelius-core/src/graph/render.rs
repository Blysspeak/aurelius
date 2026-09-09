//! Одна строка на узел — общий рендер для прозы `memory_recall` и для строк
//! блока «где остановились» (FR-010, `data-model.md` §«Грунтовка»,
//! `contracts/mcp.md` §§2.3, 3.2). Не заводит своей сущности: чистая функция
//! `Node -> Option<String>`, без обращения к базе — то же самое дерево
//! вызовов и для recall (T018/T020), и для `WorkingState::decisions`/
//! `blockers` (T025), поэтому различие узла старого формата (`window`, поле
//! `claim` в `data` отсутствует) и нового (`claim` заполнен) снаружи не
//! видно вовсе: оба идут через один и тот же `one_line`.
//!
//! Форма: `[тип] проект · дата — текст`; сегмент `проект ·` опускается
//! целиком, если у узла неоткуда его взять (см. [`project_prefix`]).

use crate::models::{Node, NodeType};
use crate::provenance::{Provenance, CLAIM_MAX_CHARS};
use crate::secret::is_secret_ref;

use super::snapshot::clip;

/// Бюджет текста строки. Не новое число: тот же потолок, что уже гарантирован
/// `claim` при записи (`provenance::CLAIM_MAX_CHARS`, `provenance.rs:32`) —
/// переиспользуется, а не изобретается заново, ровно затем, чтобы `note` и
/// `label` резались до той же ширины, до которой `claim` уже ограничен: «текст
/// берётся claim → note → label одинаково» (FR-010) иначе было бы утверждением
/// на словах, а не в коде — у `claim` `clip` при этом всегда не-операция.
const LINE_TEXT_BUDGET: usize = CLAIM_MAX_CHARS;

/// Строка прозы для узла, или `None`, если узел печатать нельзя:
///
/// - координата секрета (`secret::is_secret_ref`, FR-027) — секрет не должен
///   попасть ни в прозу, ни куда-либо ещё в автоматической выдаче;
/// - после обреза по границе слова не осталось ни одного целого слова смысла
///   (FR-011) — иначе счётчик показанных записей врал бы о том, что реально
///   показано: запись, которую нечем прочитать, не должна ни попадать в
///   прозу, ни засчитываться в `knowledge`.
pub fn one_line(node: &Node) -> Option<String> {
    if is_secret_ref(node) {
        return None;
    }

    let (project, label_rest) = match project_prefix(&node.label) {
        Some((project, rest)) => (Some(project), rest),
        None => (None, node.label.as_str()),
    };

    // FR-010: claim, иначе note, иначе label — префикс проекта из label уже
    // снят выше, чтобы не повторять его дважды в одной строке (сегмент
    // "проект ·" и так его несёт).
    let claim = Provenance::from_data(&node.data).claim;
    let text = claim
        .or_else(|| node.note.clone())
        .unwrap_or_else(|| label_rest.to_owned());

    let clipped = clip(&text, LINE_TEXT_BUDGET);
    if !has_whole_word(&text, &clipped) {
        return None;
    }

    let date = node.created_at.format("%Y-%m-%d");
    let mut line = format!("[{}]", type_name(node));
    if let Some(project) = project {
        line.push(' ');
        line.push_str(project);
        line.push_str(" ·");
    }
    line.push_str(&format!(" {date} — {clipped}"));
    Some(line)
}

/// `label` часто несёт префикс `[project] остальное` — тот же формат, что
/// пишут во множестве мест записи (`graph/session.rs:213,294,315,325`,
/// `handlers/task.rs`, `graph/mod.rs:111`, `handlers/crud.rs:609`,
/// `handlers/doc.rs:322`, `graph/pickup.rs:456`, `graph/snapshot.rs:412` —
/// везде `format!("[{project}] ...")`), и что читает `project_scope_sql`
/// (`graph/search.rs:307`: `label LIKE '[' || project || ']%'`).
///
/// Узлы, привязанные к проекту только ребром (`au note --project`,
/// `commands.rs:292`, `memory_add` без явного `--label`), эта функция не
/// видит: `one_line` — чистая функция одного узла, без обращения к базе, и
/// ребро отсюда не достать. Для таких узлов сегмент `проект ·` в строке
/// отсутствует — это ожидаемый пробел источника, а не баг рендера.
fn project_prefix(label: &str) -> Option<(&str, &str)> {
    let rest = label.strip_prefix('[')?;
    let (project, tail) = rest.split_once(']')?;
    if project.is_empty() {
        return None;
    }
    Some((project, tail.trim_start()))
}

/// Имя типа узла, как оно печатается в прозе. Та же логика, что уже есть в
/// `eval.rs` (`type_name`, приватная там и вне области этой задачи) —
/// продублирована, а не переиспользована: `graph/render.rs` не может звать
/// приватную функцию из `aurelius_core::eval`, а обе стороны, наоборот,
/// смотрят на один и тот же факт — `NodeType::Custom(name)` несёт имя внутри
/// варианта, и через serde ушёл бы в `{"custom":"run"}`, а не голой строкой.
fn type_name(node: &Node) -> String {
    match &node.node_type {
        NodeType::Custom(name) => name.clone(),
        known => serde_json::to_value(known)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
    }
}

/// Истина, если в `clipped` (результате `clip(original, LINE_TEXT_BUDGET)`)
/// осталось хотя бы одно целое слово исходного текста — условие FR-011.
///
/// `clip` (`graph/snapshot.rs:38`) режет по границе слова, когда граница есть
/// в пределах бюджета, и рубит первое слово вслепую, когда её нет (слово
/// длиннее бюджета целиком) — тогда результат несёт фрагмент без единого
/// целого слова. Пробел внутри уже обрезанного текста — надёжный признак
/// «граница была», без переигрывания внутренней арифметики `clip`; отсутствие
/// пробела разрешается сравнением длины с первым словом оригинала.
fn has_whole_word(original: &str, clipped: &str) -> bool {
    let first_word = match original.split_whitespace().next() {
        Some(w) => w,
        None => return false,
    };
    let body = clipped.strip_suffix('…').unwrap_or(clipped).trim_end();
    if body.chars().any(char::is_whitespace) {
        return true;
    }
    body.chars().count() >= first_word.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MemoryKind;
    use chrono::{DateTime, TimeZone, Utc};
    use serde_json::json;
    use uuid::Uuid;

    /// Узел с произвольными `node_type`/`label`/`note`/`data`/`created_at` и
    /// нейтральными значениями во всём остальном — тот же приём, что в
    /// `rank.rs` (`graph/rank.rs:248`), только с полями, которые нужны рендеру.
    fn node(
        node_type: NodeType,
        label: &str,
        note: Option<&str>,
        data: serde_json::Value,
        created_at: DateTime<Utc>,
    ) -> Node {
        Node {
            id: Uuid::new_v4(),
            node_type,
            label: label.to_owned(),
            note: note.map(str::to_owned),
            source: "test".to_owned(),
            data,
            created_at,
            updated_at: created_at,
            memory_kind: MemoryKind::Semantic,
            last_accessed_at: created_at,
            access_count: 0,
            content_hash: None,
            created_by: None,
            updated_by: None,
            deleted_at: None,
            sync_seq: None,
        }
    }

    fn fixed_date() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 5, 19, 41, 13)
            .single()
            .expect("валидная дата фикстуры")
    }

    // --- FR-010: одна форма для обоих форматов узла -------------------------

    #[test]
    fn window_and_claim_shaped_nodes_render_the_same_form() {
        // Старый формат: `claim` в `data` отсутствует, тело — в `note`.
        let window_shaped = node(
            NodeType::Decision,
            "[aurelius] rust-clean-code not restored (owner: no source ...",
            Some("rust-clean-code not restored; rule carried by workspace clippy"),
            json!({"confidence": "measured"}),
            fixed_date(),
        );
        // Новый формат: `claim` заполнен, `note` пуст.
        let claim_shaped = node(
            NodeType::Decision,
            "[aurelius] rust-clean-code",
            None,
            json!({"claim": "rust-clean-code not restored; rule carried by workspace clippy"}),
            fixed_date(),
        );

        let a = one_line(&window_shaped).expect("узел старого формата рендерится");
        let b = one_line(&claim_shaped).expect("узел нового формата рендерится");

        assert_eq!(a, b, "window и claim обязаны дать одну и ту же строку");
        assert_eq!(
            a,
            "[decision] aurelius · 2026-09-05 — rust-clean-code not restored; \
             rule carried by workspace clippy"
        );
    }

    // --- Цепочка полей: claim -> note -> label ------------------------------

    #[test]
    fn text_prefers_claim_over_note_over_label() {
        let claim_wins = node(
            NodeType::Concept,
            "[aurelius] лейбл",
            Some("заметка"),
            json!({"claim": "утверждение"}),
            fixed_date(),
        );
        assert!(one_line(&claim_wins)
            .expect("рендерится")
            .ends_with("— утверждение"));

        let note_wins = node(
            NodeType::Concept,
            "[aurelius] лейбл",
            Some("заметка"),
            json!({}),
            fixed_date(),
        );
        assert!(one_line(&note_wins)
            .expect("рендерится")
            .ends_with("— заметка"));

        let label_wins = node(
            NodeType::Concept,
            "[aurelius] голый лейбл",
            None,
            json!({}),
            fixed_date(),
        );
        // Префикс проекта снят из текста — он уже несётся отдельным сегментом.
        assert!(one_line(&label_wins)
            .expect("рендерится")
            .ends_with("— голый лейбл"));
    }

    // --- Проект: сегмент есть, когда есть префикс, и молчит, когда нет ------

    #[test]
    fn project_segment_present_only_with_label_prefix() {
        let with_project = node(
            NodeType::Problem,
            "[nexalix] обход глубины 2 от посева уходит в чужие проекты через узел-хаб",
            None,
            json!({}),
            Utc.with_ymd_and_hms(2026, 8, 30, 0, 0, 0)
                .single()
                .unwrap_or_else(fixed_date),
        );
        let out = one_line(&with_project).expect("рендерится");
        assert_eq!(
            out,
            "[problem] nexalix · 2026-08-30 — обход глубины 2 от посева уходит \
             в чужие проекты через узел-хаб"
        );

        // Узел, привязанный к проекту только ребром (au note --project) —
        // голый лейбл, `one_line` не открывает соединение с базой и не может
        // узнать проект: сегмент отсутствует целиком, без "·" в пустоте.
        let edge_only = node(
            NodeType::Decision,
            "решение без префикса проекта в лейбле",
            None,
            json!({}),
            fixed_date(),
        );
        let out = one_line(&edge_only).expect("рендерится");
        assert!(!out.contains('·'), "{out}");
        assert_eq!(
            out,
            "[decision] 2026-09-05 — решение без префикса проекта в лейбле"
        );
    }

    // --- Дата: YYYY-MM-DD, без времени и наносекунд -------------------------

    #[test]
    fn date_has_no_time_or_nanoseconds() {
        let precise = Utc
            .with_ymd_and_hms(2026, 9, 5, 19, 41, 13)
            .single()
            .expect("валидная дата")
            + chrono::Duration::nanoseconds(846_558_621);
        let n = node(
            NodeType::Session,
            "[aurelius] сессия",
            Some("итог"),
            json!({}),
            precise,
        );
        let out = one_line(&n).expect("рендерится");
        assert!(out.contains("2026-09-05"), "{out}");
        assert!(!out.contains(':'), "время просочилось: {out}");
        assert!(!out.contains("846"), "наносекунды просочились: {out}");
    }

    // --- UUID в прозе нет -----------------------------------------------------

    #[test]
    fn uuid_never_appears_in_prose() {
        let n = node(
            NodeType::Decision,
            "[aurelius] лейбл",
            Some("текст без идентификатора"),
            json!({}),
            fixed_date(),
        );
        let out = one_line(&n).expect("рендерится");
        assert!(!out.contains(&n.id.to_string()), "{out}");
    }

    // --- NodeType::Custom("run") печатается своим именем, не "custom" -------

    #[test]
    fn custom_type_prints_its_own_name_not_the_variant_tag() {
        let n = node(
            NodeType::Custom("run".to_owned()),
            "[aurelius] прогон: npm test",
            None,
            json!({}),
            fixed_date(),
        );
        let out = one_line(&n).expect("рендерится");
        assert!(out.starts_with("[run]"), "{out}");
    }

    // --- Координата секрета не рендерится никогда (FR-027) ------------------

    #[test]
    fn secret_ref_node_never_renders() {
        let n = node(
            NodeType::Config,
            "[aurelius] координата секрета",
            Some("не важно"),
            json!({"kind": "secret_ref"}),
            fixed_date(),
        );
        assert_eq!(one_line(&n), None);
    }

    // --- FR-011: запись без единого целого слова после обреза не рендерится ---
    //
    // Это условие, которое пин через `one_line` держит для ОБОИХ мест, где
    // его требует спека: `prose` (T018/T020) и `knowledge` (тот же список
    // узлов) обязаны согласиться на исключении одной и той же записи — а не
    // разойтись так, что запись попадёт в один список и выпадет из другого.
    // Оба списка на этом уровне строятся по одному и тому же условию
    // `one_line(node).is_some()`, поэтому `None` здесь — это гарантия
    // отсутствия записи сразу в обоих местах, а не только в одном.
    #[test]
    fn no_whole_word_survives_clip_then_node_is_dropped_entirely() {
        // Одно "слово" без единого пробела длиннее LINE_TEXT_BUDGET: `clip`
        // не находит границы и рубит его вслепую — после обреза нет ни
        // одного целого слова смысла.
        let one_giant_word = "ы".repeat(LINE_TEXT_BUDGET + 40);
        let n = node(
            NodeType::Concept,
            "[aurelius] лейбл",
            Some(&one_giant_word),
            json!({}),
            fixed_date(),
        );
        assert_eq!(
            one_line(&n),
            None,
            "запись без целого слова после обреза не должна рендериться"
        );

        // Контроль: то же самое слово, но короче бюджета, целиком влезает и
        // рендерится — значит дело именно в длине, а не в отсутствии пробелов
        // как таковых.
        let short_word = "ы".repeat(10);
        let n_short = node(
            NodeType::Concept,
            "[aurelius] лейбл",
            Some(&short_word),
            json!({}),
            fixed_date(),
        );
        let rendered = one_line(&n_short).expect("короткое слово рендерится");
        assert!(rendered.ends_with(&short_word), "{rendered}");
    }

    #[test]
    fn has_whole_word_matches_clip_boundary_behaviour() {
        let long_note = "короткое ".to_owned() + &"ы".repeat(LINE_TEXT_BUDGET + 10);
        let clipped = clip(&long_note, LINE_TEXT_BUDGET);
        // Первое слово короткое и целиком влезает до обреза — значит целое
        // слово в выдаче осталось, узел печатается.
        assert!(has_whole_word(&long_note, &clipped));

        let no_boundary = "ы".repeat(LINE_TEXT_BUDGET + 10);
        let clipped = clip(&no_boundary, LINE_TEXT_BUDGET);
        assert!(!has_whole_word(&no_boundary, &clipped));
    }
}
