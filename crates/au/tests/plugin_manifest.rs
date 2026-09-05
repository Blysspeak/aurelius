//! Contract test for the Claude Code plugin manifests shipped in this repo:
//! `plugin/.claude-plugin/plugin.json`, `plugin/hooks.json` (plugin root),
//! and `.claude-plugin/marketplace.json` (repo root). See
//! `specs/009-claude-code-plugin/contracts/plugin-layout.md`.
//!
//! The plugin root is `plugin/`, not the repo root: `claude plugin install`
//! copies the plugin root into its cache, and the repo root drags along
//! `target/` build output.
//!
//! No binary under test here — these are static data files, read directly
//! off disk relative to the repo root.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

/// Repo root: two levels up from the `au` crate manifest directory
/// (`crates/au` -> `crates` -> repo root).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Plugin root: the directory `claude plugin install` copies into its cache.
fn plugin_root() -> PathBuf {
    repo_root().join("plugin")
}

fn read_json(path: &Path) -> serde_json::Value {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {} as JSON: {e}", path.display()))
}

#[test]
fn plugin_json_version_matches_workspace() {
    let plugin_json_path = plugin_root().join(".claude-plugin/plugin.json");
    let plugin = read_json(&plugin_json_path);

    let version = plugin["version"]
        .as_str()
        .expect("plugin/.claude-plugin/plugin.json: field `version` must be a string");
    assert_eq!(
        version,
        env!("CARGO_PKG_VERSION"),
        "plugin/.claude-plugin/plugin.json: field `version` must equal [workspace.package].version"
    );
}

#[test]
fn plugin_json_registers_au_mcp_server() {
    let plugin_json_path = plugin_root().join(".claude-plugin/plugin.json");
    let plugin = read_json(&plugin_json_path);

    let server = &plugin["mcpServers"]["aurelius"];
    assert_eq!(
        server["command"].as_str(),
        Some("au"),
        "plugin/.claude-plugin/plugin.json: field `mcpServers.aurelius.command` must be \"au\""
    );
    assert_eq!(
        server["args"],
        serde_json::json!(["mcp"]),
        "plugin/.claude-plugin/plugin.json: field `mcpServers.aurelius.args` must be [\"mcp\"]"
    );
}

#[test]
fn hooks_json_has_exactly_seven_au_commands() {
    let root = plugin_root();
    let plugin_json_path = root.join(".claude-plugin/plugin.json");
    let plugin = read_json(&plugin_json_path);

    let hooks_rel = plugin["hooks"]
        .as_str()
        .expect("plugin/.claude-plugin/plugin.json: field `hooks` must be a string path");
    let hooks_json_path = root.join(hooks_rel);
    let hooks = read_json(&hooks_json_path);

    let events = hooks["hooks"]
        .as_object()
        .expect("plugin/hooks.json: field `hooks` must be an object keyed by event name");

    let mut command_count = 0usize;
    for (event, matcher_groups) in events {
        let matcher_groups = matcher_groups.as_array().unwrap_or_else(|| {
            panic!("plugin/hooks.json: event `{event}` must be an array of matcher groups")
        });
        for group in matcher_groups {
            let hook_list = group["hooks"].as_array().unwrap_or_else(|| {
                panic!("plugin/hooks.json: event `{event}` matcher group must have a `hooks` array")
            });
            for hook in hook_list {
                command_count += 1;
                assert_eq!(
                    hook["command"].as_str(),
                    Some("au"),
                    "plugin/hooks.json: event `{event}` hook must have field `command` == \"au\""
                );
                assert!(
                    hook.get("shell").is_none(),
                    "plugin/hooks.json: event `{event}` hook must not have a `shell` field"
                );
                let args = hook["args"].as_array().unwrap_or_else(|| {
                    panic!("plugin/hooks.json: event `{event}` hook must have an `args` array")
                });
                assert!(
                    !args.is_empty(),
                    "plugin/hooks.json: event `{event}` hook field `args` must not be empty"
                );
            }
        }
    }

    assert_eq!(
        command_count, 7,
        "plugin/hooks.json: total hook command count across all events must be exactly 7"
    );
}

#[test]
fn marketplace_json_lists_single_aurelius_plugin() {
    let marketplace_json_path = repo_root().join(".claude-plugin/marketplace.json");
    let marketplace = read_json(&marketplace_json_path);

    let plugins = marketplace["plugins"]
        .as_array()
        .expect(".claude-plugin/marketplace.json: field `plugins` must be an array");
    let first = plugins
        .first()
        .expect(".claude-plugin/marketplace.json: field `plugins` must have at least one entry");

    assert_eq!(
        first["name"].as_str(),
        Some("aurelius"),
        ".claude-plugin/marketplace.json: field `plugins[0].name` must be \"aurelius\""
    );
    assert_eq!(
        first["source"].as_str(),
        Some("./plugin"),
        ".claude-plugin/marketplace.json: field `plugins[0].source` must be \"./plugin\""
    );
}

/// Guards against a plugin root that drags build output into
/// `~/.claude/plugins/cache`: `claude plugin install` copies the whole
/// plugin root, so it must stay small and free of `target`/`node_modules`.
#[test]
fn plugin_root_stays_small() {
    const MAX_BYTES: u64 = 512 * 1024;
    const FORBIDDEN_NAMES: [&str; 3] = ["target", "node_modules", ".git"];

    let root = plugin_root();
    let mut total_bytes: u64 = 0;
    let mut stack = vec![root.clone()];

    while let Some(dir) = stack.pop() {
        let entries =
            std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
        for entry in entries {
            let entry =
                entry.unwrap_or_else(|e| panic!("read_dir entry in {}: {e}", dir.display()));
            let path = entry.path();
            let file_type = entry
                .file_type()
                .unwrap_or_else(|e| panic!("file_type {}: {e}", path.display()));

            if file_type.is_dir() {
                let name = entry.file_name();
                assert!(
                    !FORBIDDEN_NAMES.contains(&name.to_string_lossy().as_ref()),
                    "plugin/: forbidden directory found at {}",
                    path.display()
                );
                stack.push(path);
            } else if file_type.is_file() {
                let metadata = entry
                    .metadata()
                    .unwrap_or_else(|e| panic!("metadata {}: {e}", path.display()));
                total_bytes += metadata.len();
            }
        }
    }

    assert!(
        total_bytes < MAX_BYTES,
        "plugin/: total size {total_bytes} bytes exceeds {MAX_BYTES} byte limit"
    );
}
