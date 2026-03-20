//! MCP server over stdio.
//! Claude Code connects via: { "command": "au", "args": ["mcp"] }
//!
//! Implements tools:
//!   memory_context(topic, depth)
//!   memory_search(query)
//!   memory_add(label, type, note)
//!   memory_relate(a, b, relation)
//!   memory_dump()

use anyhow::Result;
use tracing::info;

pub async fn serve() -> Result<()> {
    info!("MCP server ready on stdio");
    // TODO: implement JSON-RPC 2.0 over stdio
    // Protocol: https://spec.modelcontextprotocol.io
    Ok(())
}
