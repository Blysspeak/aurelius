mod handlers;
mod protocol;
mod tools;

use anyhow::Result;
use protocol::{JsonRpcRequest, JsonRpcResponse, INTERNAL_ERROR, METHOD_NOT_FOUND};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, error, info};

pub async fn serve() -> Result<()> {
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

    // Run handler in spawn_blocking since rusqlite isn't Send
    let tool_name = tool_name.to_owned();
    let result = tokio::task::spawn_blocking(move || match tool_name.as_str() {
        "memory_status" => handlers::memory_status(),
        "memory_context" => handlers::memory_context(&arguments),
        "memory_search" => handlers::memory_search(&arguments),
        "memory_add" => handlers::memory_add(&arguments),
        "memory_relate" => handlers::memory_relate(&arguments),
        "memory_index" => handlers::memory_index(&arguments),
        "memory_forget" => handlers::memory_forget(&arguments),
        "memory_dump" => handlers::memory_dump(&arguments),
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
