mod handlers;
mod params;
mod protocol;
mod tools;

/// The node renderer, and nothing else out of `handlers`. `au recall` needs
/// this one function; `pub mod handlers` would have published every MCP
/// handler with it, since the module re-exports its submodules by glob.
pub use handlers::node_detail;

use anyhow::Result;
use protocol::{JsonRpcRequest, JsonRpcResponse, INTERNAL_ERROR, METHOD_NOT_FOUND};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, error, info};

pub async fn serve() -> Result<()> {
    // Recorded before the request loop starts: `memory_status` compares this
    // against the running binary's own mtime to detect a stale image (see
    // `handlers::restart_needed`).
    handlers::mark_server_started();

    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    info!("MCP server ready on stdio");

    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_owned();
        if line.is_empty() {
            continue;
        }

        debug!("recv: {line}");

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse::error(None, -32700, format!("Parse error: {e}"));
                write_response(&mut stdout, &resp).await?;
                continue;
            }
        };

        let response = dispatch(request).await;

        if let Some(resp) = response {
            write_response(&mut stdout, &resp).await?;
        }
    }

    Ok(())
}

async fn dispatch(req: JsonRpcRequest) -> Option<JsonRpcResponse> {
    let id = req.id.clone();

    // Notifications (no id) don't get responses
    let is_notification = id.is_none();

    let result = match req.method.as_str() {
        "initialize" => Some(handle_initialize(id.clone())),
        "notifications/initialized" => None, // notification, no response
        "tools/list" => Some(handle_tools_list(id.clone())),
        "tools/call" => Some(handle_tools_call(id.clone(), &req.params).await),
        _ => {
            if is_notification {
                None
            } else {
                Some(JsonRpcResponse::error(
                    id,
                    METHOD_NOT_FOUND,
                    format!("Unknown method: {}", req.method),
                ))
            }
        }
    };

    result
}

fn handle_initialize(id: Option<serde_json::Value>) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "aurelius",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    )
}

fn handle_tools_list(id: Option<serde_json::Value>) -> JsonRpcResponse {
    JsonRpcResponse::success(id, tools::tool_definitions())
}

async fn handle_tools_call(
    id: Option<serde_json::Value>,
    params: &serde_json::Value,
) -> JsonRpcResponse {
    let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    debug!("tool call: {tool_name}");

    // Заслонка ДО обработчика: перевранное имя параметра обязано остановить
    // вызов целиком, а не потерять половину записи молча. Отвечаем как об
    // ошибке ИНСТРУМЕНТА (isError), а не протокола: клиент показывает такую
    // модели, и она может перезвать правильно — протокольная до неё не дойдёт.
    if let Err(e) = params::validate(tool_name, &arguments) {
        error!("rejected call: {e}");
        return JsonRpcResponse::success(
            id,
            serde_json::json!({
                "content": [{ "type": "text", "text": format!("Error: {e:#}") }],
                "isError": true
            }),
        );
    }

    // Run handler in spawn_blocking since rusqlite isn't Send
    let tool_name = tool_name.to_owned();
    let result = tokio::task::spawn_blocking(move || match tool_name.as_str() {
        "memory_status" => handlers::memory_status(&arguments),
        "memory_context" => handlers::memory_context(&arguments),
        "memory_path" => handlers::memory_path(&arguments),
        "memory_search" => handlers::memory_search(&arguments),
        "memory_add" => handlers::memory_add(&arguments),
        "memory_relate" => handlers::memory_relate(&arguments),
        "memory_index" => handlers::memory_index(&arguments),
        "memory_forget" => handlers::memory_forget(&arguments),
        "memory_update" => handlers::memory_update(&arguments),
        "memory_session" => handlers::memory_session(&arguments),
        "memory_recall" => handlers::memory_recall(&arguments),
        "memory_dump" => handlers::memory_dump(&arguments),
        "memory_gc" => handlers::memory_gc(),
        "memory_merge" => handlers::memory_merge(&arguments),
        "memory_snapshot" => handlers::memory_snapshot(&arguments),
        "memory_consolidate" => handlers::memory_consolidate(&arguments),
        "task_create" => handlers::task_create(&arguments),
        "task_update" => handlers::task_update(&arguments),
        "task_list" => handlers::task_list(&arguments),
        "task_log" => handlers::task_log(&arguments),
        "task_view" => handlers::task_view(&arguments),
        "task_stats" => handlers::task_stats(&arguments),
        "task_ripe" => handlers::task_ripe(&arguments),
        "secret_list" => handlers::secret_list(&arguments),
        "search_web" => handlers::search_web(&arguments),
        "search_recall" => handlers::search_recall(&arguments),
        "doc_convert" => handlers::doc_convert(&arguments),
        "doc_read" => handlers::doc_read(&arguments),
        "doc_recall" => handlers::doc_recall(&arguments),
        "skill_list" => handlers::skill_list(&arguments),
        "skill_get" => handlers::skill_get(&arguments),
        "skill_save" => handlers::skill_save(&arguments),
        "skill_remove" => handlers::skill_remove(&arguments),
        _ => Err(anyhow::anyhow!("Unknown tool: {tool_name}")),
    })
    .await;

    match result {
        Ok(Ok(value)) => JsonRpcResponse::success(
            id,
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&value).unwrap_or_default()
                }]
            }),
        ),
        Ok(Err(e)) => {
            error!("tool error: {e}");
            JsonRpcResponse::success(
                id,
                serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Error: {e}")
                    }],
                    "isError": true
                }),
            )
        }
        Err(e) => {
            error!("spawn error: {e}");
            JsonRpcResponse::error(id, INTERNAL_ERROR, format!("Internal error: {e}"))
        }
    }
}

async fn write_response(stdout: &mut tokio::io::Stdout, resp: &JsonRpcResponse) -> Result<()> {
    let json = serde_json::to_string(resp)?;
    debug!("send: {json}");
    stdout.write_all(json.as_bytes()).await?;
    stdout.write_all(b"\n").await?;
    stdout.flush().await?;
    Ok(())
}
