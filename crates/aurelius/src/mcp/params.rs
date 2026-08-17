//! Заслонка на параметры вызова: неизвестное имя — ошибка, а не молчание.
//!
//! Повод конкретный. `memory_session` дважды позвали с перевранными именами
//! параметров: ответ был `created: true`, решения и next_steps потерялись, а в
//! графе остались узлы-сироты с меткой `[unknown]`. Единственным признаком беды
//! была подстрока в метке. Это ровно тот класс «выглядит применённым, но не
//! применено», который дороже всего: ошибка обнаруживается не при вызове, а
//! через неделю, когда за неё уже нельзя зацепиться.
//!
//! Список известных имён берётся из тех же `inputSchema`, что отдаются клиенту
//! в `tools/list`. Отдельная копия разъехалась бы с ними — в этом же проекте
//! описание связей уже отставало от словаря на два значения.

use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::{bail, Result};

use super::tools::tool_definitions;

struct Schema {
    known: Vec<String>,
    required: Vec<String>,
    /// Допустимые значения там, где схема их перечисляет. Без этой проверки
    /// опечатка в значении проходила молча: `NodeType::parse` заводит на
    /// неизвестном имени `Custom(...)`, и узел получал тип, которого не ищет ни
    /// одна выборка, — тогда как CLI такую же опечатку отвергает.
    enums: HashMap<String, Vec<String>>,
}

fn schemas() -> &'static HashMap<String, Schema> {
    static SCHEMAS: OnceLock<HashMap<String, Schema>> = OnceLock::new();
    SCHEMAS.get_or_init(|| {
        let defs = tool_definitions();
        let mut map = HashMap::new();
        let Some(tools) = defs.get("tools").and_then(|t| t.as_array()) else {
            return map;
        };
        for tool in tools {
            let Some(name) = tool.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            let schema = tool.get("inputSchema");
            let properties = schema
                .and_then(|s| s.get("properties"))
                .and_then(|p| p.as_object());
            let known = properties
                .map(|p| p.keys().cloned().collect())
                .unwrap_or_default();
            let enums = properties
                .map(|p| {
                    p.iter()
                        .filter_map(|(key, spec)| {
                            let values: Vec<String> = spec
                                .get("enum")?
                                .as_array()?
                                .iter()
                                .filter_map(|v| v.as_str().map(str::to_owned))
                                .collect();
                            (!values.is_empty()).then(|| (key.clone(), values))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let required = schema
                .and_then(|s| s.get("required"))
                .and_then(|r| r.as_array())
                .map(|r| {
                    r.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            map.insert(
                name.to_owned(),
                Schema {
                    known,
                    required,
                    enums,
                },
            );
        }
        map
    })
}

/// Расстояние Левенштейна — чтобы на опечатку отвечать подсказкой, а не
/// списком из двенадцати имён, в котором нужное ещё надо высмотреть.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Ближайшее известное имя, если оно достаточно близко, чтобы быть опечаткой.
/// Порог — треть длины: иначе на `foo` предлагалось бы `project`.
fn nearest<'a>(unknown: &str, known: &'a [String]) -> Option<&'a str> {
    let limit = (unknown.chars().count() / 3).max(1);
    known
        .iter()
        .map(|k| (distance(unknown, k), k.as_str()))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, k)| k)
}

/// Проверить параметры вызова против объявленной схемы инструмента.
///
/// # Errors
/// Неизвестное имя параметра или отсутствие обязательного. Инструмент, которого
/// нет в схемах, пропускается: судить о нём нечем, и выдумывать приговор хуже,
/// чем позволить обработчику ответить самому.
pub fn validate(tool: &str, params: &serde_json::Value) -> Result<()> {
    let Some(schema) = schemas().get(tool) else {
        return Ok(());
    };
    let Some(given) = params.as_object() else {
        return Ok(());
    };

    // Перечисляем ВСЕ неизвестные имена разом. Отказ по первому заставил бы
    // чинить вызов по одному имени за раз, а перевирают обычно сразу несколько.
    let unknown: Vec<String> = given
        .keys()
        .filter(|k| !schema.known.iter().any(|known| known == *k))
        .map(|k| match nearest(k, &schema.known) {
            Some(suggestion) => format!("'{k}' (похоже на '{suggestion}')"),
            None => format!("'{k}'"),
        })
        .collect();
    if !unknown.is_empty() {
        bail!(
            "{tool}: неизвестные параметры: {}. Известные: {}. \
             Ничего не записано — повтори вызов с правильными именами",
            unknown.join(", "),
            schema.known.join(", ")
        );
    }

    let missing: Vec<&str> = schema
        .required
        .iter()
        .filter(|r| !given.contains_key(r.as_str()))
        .map(String::as_str)
        .collect();
    if !missing.is_empty() {
        bail!(
            "{tool}: не хватает обязательных параметров: {}",
            missing.join(", ")
        );
    }

    for (key, allowed) in &schema.enums {
        let Some(value) = given.get(key).and_then(|v| v.as_str()) else {
            continue;
        };
        if !allowed.iter().any(|a| a == value) {
            bail!(
                "{tool}: недопустимое значение '{value}' у '{key}'. Допустимые: {}. \
                 Ничего не записано",
                allowed.join(", ")
            );
        }
    }

    Ok(())
}

/// Что из переданного действительно легло, а что оказалось пустым.
///
/// Имена теперь проверены, но остаётся вторая половина той же беды: параметр с
/// правильным именем и пустым значением выглядит переданным. `decisions: []`
/// молча не записывает ни одного решения — и вызывающий узнаёт об этом, только
/// заглянув в граф.
#[must_use]
pub fn field_report(params: &serde_json::Value) -> (Vec<String>, Vec<String>) {
    let mut stored = Vec::new();
    let mut dropped = Vec::new();
    let Some(given) = params.as_object() else {
        return (stored, dropped);
    };
    for (key, value) in given {
        let empty = match value {
            serde_json::Value::Null => true,
            serde_json::Value::String(s) => s.trim().is_empty(),
            serde_json::Value::Array(a) => a.is_empty(),
            serde_json::Value::Object(o) => o.is_empty(),
            _ => false,
        };
        if empty {
            dropped.push(key.clone());
        } else {
            stored.push(key.clone());
        }
    }
    stored.sort();
    dropped.sort();
    (stored, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_misspelled_parameter_is_refused_with_a_suggestion() {
        let err = validate(
            "memory_session",
            &json!({ "sumary": "итог", "project": "p" }),
        )
        .expect_err("опечатка обязана быть ошибкой");
        let msg = format!("{err}");
        assert!(msg.contains("summary"), "нужна подсказка: {msg}");
        assert!(msg.contains("Ничего не записано"), "{msg}");
    }

    /// Ровно сегодняшний случай: имена перевраны целиком, ответ был created:true.
    #[test]
    fn wholly_wrong_names_do_not_pass_as_an_empty_call() {
        let err = validate(
            "memory_session",
            &json!({ "what_happened": "итог", "made": ["решение"] }),
        )
        .expect_err("перевранные имена — не пустой вызов");
        let msg = format!("{err}");
        // Оба имени разом: чинить вызов по одному имени за заход — та же потеря
        // времени, что и молчаливый пропуск.
        assert!(msg.contains("what_happened"), "{msg}");
        assert!(msg.contains("made"), "{msg}");
    }

    #[test]
    fn a_missing_required_parameter_is_refused() {
        let err =
            validate("memory_add", &json!({ "label": "факт" })).expect_err("confidence обязателен");
        assert!(format!("{err}").contains("confidence"), "{err}");
    }

    #[test]
    fn a_correct_call_passes() {
        validate(
            "memory_add",
            &json!({ "label": "факт", "confidence": "reported", "project": "aurelius" }),
        )
        .expect("правильный вызов");
    }

    #[test]
    fn an_unknown_tool_is_left_to_its_handler() {
        validate("нет_такого", &json!({ "что_угодно": 1 })).expect("судить нечем");
    }

    /// Опечатка в ЗНАЧЕНИИ так же дорога, как в имени: NodeType::parse заводит
    /// на неизвестном имени Custom(...), и узел получает тип, которого не ищет
    /// ни одна выборка.
    #[test]
    fn a_misspelled_enum_value_is_refused() {
        let err = validate(
            "memory_add",
            &json!({ "label": "факт", "confidence": "reported", "type": "десижн" }),
        )
        .expect_err("несуществующий тип узла");
        assert!(format!("{err}").contains("decision"), "{err}");
    }

    #[test]
    fn an_empty_value_is_reported_as_dropped() {
        let (stored, dropped) = field_report(&json!({
            "summary": "итог",
            "decisions": [],
            "next_steps": ["дальше"],
            "key_files": "",
        }));
        assert_eq!(stored, ["next_steps", "summary"]);
        assert_eq!(dropped, ["decisions", "key_files"]);
    }
}
