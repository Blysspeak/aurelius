use anyhow::Result;
use aurelius_core::graph;
use serde_json::json;

use super::open_db;

/// Координаты секретов проекта — то же чтение, что и `au secret list`
/// (`graph::list_secret_refs`), доступное ассистенту через MCP.
///
/// Смысл US4 (спека 007) в том, чтобы на вопрос «где лежит ключ Stripe»
/// отвечать местом, а не отправлять искать по файлам. Координата намеренно
/// не попадает ни в `memory_snapshot`, ни в другую автоматическую выгрузку
/// (тот же принцип FR-025, что запрещает хранить само значение секрета —
/// координата тоже не должна светиться там, где её не спрашивали), поэтому
/// раньше её было не достать иначе как из терминала человеком. Здесь — то
/// же чтение по явному запросу, а не автоматическая выгрузка: значения
/// секрета в ответе нет и не может быть, `add_secret_ref` его никогда не
/// принимает.
pub fn secret_list(params: &serde_json::Value) -> Result<serde_json::Value> {
    let conn = open_db()?;
    secret_list_with_conn(&conn, params)
}

/// Тело `secret_list` с явным соединением — тот же приём тестируемости, что
/// и у `task_update_with_conn`/`task_view_with_conn`.
fn secret_list_with_conn(
    conn: &rusqlite::Connection,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let project = params.get("project").and_then(|p| p.as_str());
    let refs = graph::list_secret_refs(conn, project)?;
    let items: Vec<serde_json::Value> = refs
        .iter()
        .map(|n| {
            json!({
                "name": n.data.get("name"),
                "purpose": n.data.get("purpose"),
                "location": n.data.get("location"),
                "location_kind": n.data.get("location_kind"),
            })
        })
        .collect();
    Ok(json!({
        "secrets": items,
        "total": items.len(),
        "project": project,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurelius_core::db;

    /// Тот же приём, что и в `handlers::task` тестах: настоящий temp-файл, не
    /// `:memory:` — `db::open` жёстко требует WAL.
    struct TmpDb(std::path::PathBuf);

    impl TmpDb {
        fn new(tag: &str) -> Self {
            Self(std::env::temp_dir().join(format!(
                "aurelius-mcp-secret-test-{tag}-{}.db",
                uuid::Uuid::new_v4()
            )))
        }
    }

    impl Drop for TmpDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let mut p = self.0.as_os_str().to_owned();
                p.push(suffix);
                let _ = std::fs::remove_file(std::path::PathBuf::from(p));
            }
        }
    }

    fn setup() -> (TmpDb, rusqlite::Connection) {
        let tmp = TmpDb::new("setup");
        let conn = db::open(&tmp.0).expect("open temp db");
        (tmp, conn)
    }

    /// Координата отдаётся именем, местом, назначением и разобранным видом
    /// места — но никогда значением: `add_secret_ref` его вообще не
    /// принимает, а этот тест проверяет, что и ответ ручки не сочиняет его
    /// из ничего (в JSON нет ключа `value`/`secret`).
    #[test]
    fn secret_list_returns_coordinates_never_a_value() {
        let (_tmp, conn) = setup();
        graph::add_secret_ref(
            &conn,
            Some("proj-secret"),
            "STRIPE_SECRET_KEY",
            Some("charge webhooks"),
            "env:STRIPE_SECRET_KEY",
        )
        .expect("add secret ref");

        let result =
            secret_list_with_conn(&conn, &json!({"project": "proj-secret"})).expect("secret_list");

        assert_eq!(result["total"], json!(1));
        let entry = &result["secrets"][0];
        assert_eq!(entry["name"], "STRIPE_SECRET_KEY");
        assert_eq!(entry["purpose"], "charge webhooks");
        assert_eq!(entry["location"], "env:STRIPE_SECRET_KEY");
        assert_eq!(entry["location_kind"], "env");
        assert!(entry.get("value").is_none());
    }

    /// Асимметрия предыдущего теста: проект без единой координаты — пустой
    /// список, а не ошибка и не координаты чужого проекта.
    #[test]
    fn secret_list_returns_empty_for_project_without_secrets() {
        let (_tmp, conn) = setup();
        graph::add_secret_ref(
            &conn,
            Some("proj-other"),
            "OTHER_KEY",
            None,
            "env:OTHER_KEY",
        )
        .expect("add secret ref");

        let result =
            secret_list_with_conn(&conn, &json!({"project": "proj-empty"})).expect("secret_list");

        assert_eq!(result["total"], json!(0));
        assert!(result["secrets"].as_array().expect("array").is_empty());
    }
}
