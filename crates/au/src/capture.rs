//! Ловля факта в момент открытия, а не перед компакцией.
//!
//! Хук конца сессии кричит «сохранись» тогда, когда контекст уже деградировал,
//! и память пишется по воспоминанию о том, что было. Здесь — противоположный
//! момент: команда только что вернула данные, вывод ещё перед глазами, и та
//! самая команда уже готова лечь в `evidence` дословно.
//!
//! Хук ничего не пишет сам. Отвергнутая альтернатива — автоматически заводить
//! measured-факт на каждый удачный `psql` — превратила бы граф в свалку: команда
//! с выводом ещё не факт, который стоит помнить. Поэтому хук только подсказывает,
//! а решает модель.

/// Опознанная добыча данных: чем опознали, что выполняли, сколько строк вернуло.
pub struct Catch {
    pub tool: &'static str,
    pub command: String,
    pub lines: usize,
}

/// Команды, чей вывод — это состояние мира, а не состояние репозитория.
///
/// Список намеренно узкий. Хук, срабатывающий на всём подряд, читается один
/// день, а потом игнорируется — ровно то, чем сегодня плох `probe_warnings`.
const DATA_TOOLS: &[&str] = &[
    "psql",
    "mysql",
    "mariadb",
    "sqlite3",
    "mongosh",
    "redis-cli",
    "ssh",
    "curl",
    "wget",
    "kubectl",
    "aws",
    "gcloud",
    "az",
    "docker",
    "systemctl",
    "journalctl",
    "dig",
    "nslookup",
];

/// Чтение файла считается добычей только для конфигов и секретов: `cat` по
/// исходнику — обычная навигация по коду, и предлагать на неё запись значит
/// шуметь на каждом шагу.
const READERS: &[&str] = &["cat", "head", "tail", "less", "type"];

fn looks_like_config(arg: &str) -> bool {
    let a = arg.to_ascii_lowercase();
    let a = a.trim_matches(|c| c == '"' || c == '\'');
    a.contains(".env")
        || a.ends_with(".conf")
        || a.ends_with(".ini")
        || a.ends_with(".toml")
        || a.ends_with(".yaml")
        || a.ends_with(".yml")
        || a.contains("secret")
        || a.contains("credential")
}

/// Путь до бинаря сводится к имени: `/usr/bin/psql` и `psql` — одно и то же.
fn basename(token: &str) -> &str {
    token.rsplit(['/', '\\']).next().unwrap_or(token)
}

/// Докуда искать имя команды в сегменте. Искать в первой же лексеме мало:
/// у обёрток свои аргументы (`sudo -u postgres psql`). Искать по всему
/// сегменту — много: инструмент, упомянутый в тексте аргумента, дал бы ложное
/// срабатывание. Команда всегда стоит в начале, четырёх лексем ей хватает.
const HEAD_SCAN: usize = 4;

/// Опознать, добывала ли команда данные о мире. `None` — не добывала.
///
/// Цепочки и конвейеры разбираются посегментно: `ssh host 'psql -c …'`
/// и `kubectl … | jq` должны опознаваться так же, как одиночная команда.
#[must_use]
pub fn classify(command: &str) -> Option<&'static str> {
    for segment in command.split(['|', ';', '\n']).flat_map(|s| s.split("&&")) {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        let Some(first) = tokens.first() else {
            continue;
        };
        // Собственный инструмент пропускаем явно: подсказка предлагает `au note
        // --evidence "psql …"`, и без этой отсечки хук ловил бы сам себя.
        if basename(first) == "au" {
            continue;
        }
        // `--version` и `--help` ничего не рассказывают о мире.
        if tokens.iter().any(|a| *a == "--version" || *a == "--help") {
            continue;
        }
        for (i, token) in tokens.iter().enumerate().take(HEAD_SCAN) {
            let head = basename(token);
            if let Some(t) = DATA_TOOLS.iter().find(|t| **t == head) {
                return Some(t);
            }
            if READERS.contains(&head) && tokens[i + 1..].iter().any(|a| looks_like_config(a)) {
                return Some("config");
            }
        }
    }
    None
}

/// Разобрать событие PostToolUse. `None` — предлагать нечего.
#[must_use]
pub fn from_hook_event(event: &serde_json::Value) -> Option<Catch> {
    let tool_name = event
        .get("tool_name")
        .and_then(|t| t.as_str())
        .unwrap_or("");
    if !matches!(tool_name, "Bash" | "PowerShell") {
        return None;
    }
    let command = event
        .get("tool_input")
        .and_then(|i| i.get("command"))
        .and_then(|c| c.as_str())?
        .trim();
    let tool = classify(command)?;

    // Пустой вывод — нечего сохранять. Здесь же отсекаются падения: команда,
    // сломавшаяся на подключении, ничего о мире не сообщила.
    let response = event.get("tool_response");
    let stdout = response
        .and_then(|r| r.get("stdout"))
        .and_then(|s| s.as_str())
        .or_else(|| response.and_then(serde_json::Value::as_str))
        .unwrap_or("");
    let lines = stdout.lines().filter(|l| !l.trim().is_empty()).count();
    if lines == 0 {
        return None;
    }

    Some(Catch {
        tool,
        command: command.to_owned(),
        lines,
    })
}

/// Сколько символов команды доходит до подсказки. Дословность важнее краткости,
/// но простыня на несколько килобайт в контексте — это уже не подсказка.
const EVIDENCE_MAX: usize = 2_000;

/// Текст подсказки: та же команда, уже подставленная в `evidence`.
#[must_use]
pub fn suggestion(catch: &Catch) -> String {
    let mut cmd: String = catch.command.chars().take(EVIDENCE_MAX).collect();
    if catch.command.chars().count() > EVIDENCE_MAX {
        cmd.push_str(" …(обрезано)");
    }
    let escaped = cmd.replace('"', "\\\"");
    // Счётчик строк осмыслен только там, где вывод действительно был: при
    // проверке команды руками «строк: 0» — не факт о выводе, а шум.
    let source = match catch.lines {
        0 => catch.tool.to_owned(),
        n => format!("{}, строк: {n}", catch.tool),
    };
    format!(
        "Команда вернула данные о мире ({source}). Если из вывода следует факт, \
         который переживёт эту сессию, — запиши его СЕЙЧАС, пока вывод перед глазами, \
         а не перед компакцией по памяти:\n\n\
         au note \"<что именно выяснилось>\" --claim \"<то же в одну строку>\" \
         --confidence measured --evidence \"{escaped}\" \
         --volatility <immutable|slow|volatile> --verify-with \"{escaped}\" \
         --subject <проект:предмет> --project <проект>\n\n\
         Факта нет — молчи, повторно об этой команде не напомню."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn data_commands_are_recognised_through_pipes_and_wrappers() {
        assert_eq!(classify("psql -c 'select 1'"), Some("psql"));
        assert_eq!(
            classify("sudo -u postgres psql -c 'select 1'"),
            Some("psql")
        );
        assert_eq!(classify("PGPASSWORD=x /usr/bin/psql -c 'x'"), Some("psql"));
        assert_eq!(classify("kubectl get pods | head -20"), Some("kubectl"));
        assert_eq!(classify("ls && curl -s https://api"), Some("curl"));
    }

    #[test]
    fn ordinary_work_stays_silent() {
        // Главный риск здесь — шум: хук, срабатывающий на всём, не читают.
        assert_eq!(classify("cargo test --workspace"), None);
        assert_eq!(classify("git log --oneline -20"), None);
        assert_eq!(classify("cat src/main.rs"), None);
        assert_eq!(classify("psql --version"), None);
        assert_eq!(classify("au note \"psql вернул 5\""), None);
    }

    #[test]
    fn reading_a_config_counts_but_reading_code_does_not() {
        assert_eq!(classify("cat /home/xhub/app/.env"), Some("config"));
        assert_eq!(classify("tail -5 /etc/nginx/nginx.conf"), Some("config"));
        assert_eq!(classify("head -20 crates/au/src/main.rs"), None);
    }

    #[test]
    fn silent_output_yields_nothing_to_save() {
        let event = json!({
            "tool_name": "Bash",
            "tool_input": {"command": "psql -c 'select 1'"},
            "tool_response": {"stdout": "   \n\n"}
        });
        assert!(from_hook_event(&event).is_none());
    }

    #[test]
    fn the_command_itself_lands_in_evidence_verbatim() {
        let event = json!({
            "tool_name": "Bash",
            "tool_input": {"command": "psql -c \"select count(*) from orders\""},
            "tool_response": {"stdout": "count\n-----\n42\n"}
        });
        let catch = from_hook_event(&event).expect("psql с выводом должен опознаться");
        assert_eq!(catch.lines, 3);
        let text = suggestion(&catch);
        assert!(text.contains("--evidence \"psql -c \\\"select count(*) from orders\\\"\""));
        assert!(text.contains("--confidence measured"));
    }

    #[test]
    fn other_tools_are_none_of_our_business() {
        let event = json!({
            "tool_name": "Edit",
            "tool_input": {"command": "psql -c 'select 1'"},
            "tool_response": {"stdout": "ok"}
        });
        assert!(from_hook_event(&event).is_none());
    }
}
