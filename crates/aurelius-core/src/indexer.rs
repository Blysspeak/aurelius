use crate::graph;
use crate::models::{MemoryKind, NodeType, Relation};
use anyhow::{Context, Result};
use ignore::WalkBuilder;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug)]
pub struct IndexResult {
    pub project_name: String,
    pub crates_found: usize,
    pub files_indexed: usize,
    pub dependencies_found: usize,
    pub nodes_created: usize,
    pub nodes_updated: usize,
    pub nodes_removed: usize,
}

/// Auto-index the project at `path` if it hasn't been indexed yet.
/// Returns true if indexing was performed, false if project already existed.
pub fn ensure_indexed(conn: &rusqlite::Connection, path: &Path) -> Result<bool> {
    let path = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return Ok(false),
    };
    let project_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    // Already indexed?
    if graph::find_project_by_label(conn, project_name)?.is_some() {
        return Ok(false);
    }

    // Has Cargo.toml or package.json? If not, not a project root — skip.
    if !path.join("Cargo.toml").exists() && !path.join("package.json").exists() {
        return Ok(false);
    }

    index_project(conn, &path)?;
    Ok(true)
}

pub fn index_project(conn: &rusqlite::Connection, path: &Path) -> Result<IndexResult> {
    let path = path
        .canonicalize()
        .context("Cannot canonicalize project path")?;
    let project_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_owned();

    let mut result = IndexResult {
        project_name: project_name.clone(),
        crates_found: 0,
        files_indexed: 0,
        dependencies_found: 0,
        nodes_created: 0,
        nodes_updated: 0,
        nodes_removed: 0,
    };

    let path_str = path.to_string_lossy().to_string();

    // Detect project type
    let cargo_toml = path.join("Cargo.toml");
    if !cargo_toml.exists() {
        // Non-Rust project: create a basic Project node and index key files
        let project_node = get_or_create_project(conn, &project_name, &path_str, &mut result)?;
        index_generic_files(conn, &path, project_node.id, &mut result)?;
        return Ok(result);
    }

    // Rust project
    let project_node = get_or_create_project(conn, &project_name, &path_str, &mut result)?;

    // Parse root Cargo.toml
    let cargo_content =
        std::fs::read_to_string(&cargo_toml).context("Failed to read root Cargo.toml")?;
    let cargo: toml::Value = cargo_content
        .parse()
        .context("Failed to parse root Cargo.toml")?;

    // Index root Cargo.toml as a file
    index_file(conn, &cargo_toml, project_node.id, &mut result)?;

    // Check for workspace members
    let members = extract_workspace_members(&cargo, &path);

    if members.is_empty() {
        // Single-crate project
        index_rust_crate(conn, &path, &project_name, project_node.id, &mut result)?;
    } else {
        // Workspace project
        for member_path in &members {
            let crate_toml = member_path.join("Cargo.toml");
            if !crate_toml.exists() {
                continue;
            }
            let crate_name = member_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_owned();
            index_rust_crate(conn, member_path, &crate_name, project_node.id, &mut result)?;
        }
    }

    // Clean up deleted files
    cleanup_deleted_files(conn, &path_str, &mut result)?;

    Ok(result)
}

fn get_or_create_project(
    conn: &rusqlite::Connection,
    name: &str,
    path_str: &str,
    result: &mut IndexResult,
) -> Result<crate::models::Node> {
    // Look for existing project node by path
    if let Some(existing) = graph::find_node_by_data_field(conn, "path", path_str)? {
        return Ok(existing);
    }

    let node = graph::add_node(
        conn,
        NodeType::Project,
        name,
        None,
        "indexer",
        serde_json::json!({ "path": path_str, "type": "rust" }),
    )?;
    result.nodes_created += 1;
    Ok(node)
}

fn extract_workspace_members(cargo: &toml::Value, root: &Path) -> Vec<std::path::PathBuf> {
    let mut paths = vec![];
    if let Some(members) = cargo
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
    {
        for member in members {
            if let Some(pattern) = member.as_str() {
                // Handle glob patterns like "crates/*"
                if pattern.contains('*') {
                    let base = root.join(pattern.split('*').next().unwrap_or(""));
                    if base.is_dir() {
                        if let Ok(entries) = std::fs::read_dir(&base) {
                            for entry in entries.flatten() {
                                if entry.path().join("Cargo.toml").exists() {
                                    paths.push(entry.path());
                                }
                            }
                        }
                    }
                } else {
                    let member_path = root.join(pattern);
                    if member_path.exists() {
                        paths.push(member_path);
                    }
                }
            }
        }
    }
    paths
}

fn index_rust_crate(
    conn: &rusqlite::Connection,
    crate_path: &Path,
    crate_name: &str,
    project_id: uuid::Uuid,
    result: &mut IndexResult,
) -> Result<()> {
    let crate_path_str = crate_path.to_string_lossy().to_string();

    // Get or create crate node
    let crate_node =
        if let Some(existing) = graph::find_node_by_data_field(conn, "path", &crate_path_str)? {
            existing
        } else {
            let crate_toml_path = crate_path.join("Cargo.toml");
            let description = if crate_toml_path.exists() {
                let content = std::fs::read_to_string(&crate_toml_path).unwrap_or_default();
                let parsed: toml::Value = content
                    .parse()
                    .unwrap_or(toml::Value::Table(Default::default()));
                parsed
                    .get("package")
                    .and_then(|p| p.get("description"))
                    .and_then(|d| d.as_str())
                    .map(str::to_owned)
            } else {
                None
            };

            let node = graph::add_node(
                conn,
                NodeType::Crate,
                crate_name,
                description.as_deref(),
                "indexer",
                serde_json::json!({ "path": crate_path_str }),
            )?;
            result.nodes_created += 1;

            // Project -> Contains -> Crate
            graph::add_edge(conn, project_id, node.id, Relation::Contains, 1.0)?;

            node
        };
    result.crates_found += 1;

    // Parse crate dependencies
    let crate_toml = crate_path.join("Cargo.toml");
    if crate_toml.exists() {
        index_crate_dependencies(conn, &crate_toml, crate_node.id, result)?;
        index_file(conn, &crate_toml, crate_node.id, result)?;
    }

    // Index key source files
    let key_files = ["src/main.rs", "src/lib.rs"];
    for fname in &key_files {
        let fpath = crate_path.join(fname);
        if fpath.exists() {
            index_file(conn, &fpath, crate_node.id, result)?;
        }
    }

    Ok(())
}

fn index_crate_dependencies(
    conn: &rusqlite::Connection,
    toml_path: &Path,
    crate_id: uuid::Uuid,
    result: &mut IndexResult,
) -> Result<()> {
    let content = std::fs::read_to_string(toml_path)?;
    let parsed: toml::Value = content.parse()?;

    if let Some(deps) = parsed.get("dependencies").and_then(|d| d.as_table()) {
        for dep_name in deps.keys() {
            // Skip path dependencies (workspace members)
            let is_path_dep = deps[dep_name]
                .as_table()
                .map(|t| t.contains_key("path"))
                .unwrap_or(false);
            if is_path_dep {
                continue;
            }

            // Get or create dependency node
            let dep_node = if let Some(existing) = graph::find_node_by_label(conn, dep_name)? {
                existing
            } else {
                let node = graph::add_node(
                    conn,
                    NodeType::Dependency,
                    dep_name,
                    None,
                    "indexer",
                    serde_json::json!({}),
                )?;
                result.nodes_created += 1;
                node
            };

            // Check if edge already exists (simple dedup by checking graph)
            graph::add_edge(conn, crate_id, dep_node.id, Relation::DependsOn, 1.0)?;
            result.dependencies_found += 1;
        }
    }

    Ok(())
}

fn index_file(
    conn: &rusqlite::Connection,
    file_path: &Path,
    parent_id: uuid::Uuid,
    result: &mut IndexResult,
) -> Result<()> {
    let content = std::fs::read_to_string(file_path).unwrap_or_default();
    let hash = compute_hash(&content);
    let path_str = file_path.to_string_lossy().to_string();

    // Check if file node already exists
    if let Some(existing) = graph::find_node_by_data_field(conn, "path", &path_str)? {
        // Compare hash
        if existing.content_hash.as_deref() == Some(&hash) {
            return Ok(()); // No changes
        }
        // Update existing node
        graph::update_node(
            conn,
            existing.id,
            None,
            Some(serde_json::json!({ "path": path_str })),
        )?;
        result.nodes_updated += 1;
    } else {
        let fname = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let node = graph::add_node_full(
            conn,
            NodeType::File,
            fname,
            None,
            "indexer",
            serde_json::json!({ "path": path_str }),
            MemoryKind::Semantic,
            Some(&hash),
        )?;
        result.nodes_created += 1;

        // Parent -> Contains -> File
        graph::add_edge(conn, parent_id, node.id, Relation::Contains, 0.5)?;
    }

    result.files_indexed += 1;
    Ok(())
}

fn index_generic_files(
    conn: &rusqlite::Connection,
    root: &Path,
    project_id: uuid::Uuid,
    result: &mut IndexResult,
) -> Result<()> {
    let walker = WalkBuilder::new(root).max_depth(Some(3)).build();

    for entry in walker.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Index key config/doc files (skip Cargo.toml — this is a non-Rust project)
        let is_key = matches!(
            fname,
            "package.json" | "pyproject.toml" | "go.mod" | "Makefile" | "Dockerfile"
                | "tsconfig.json" | "docker-compose.yml" | "docker-compose.yaml"
        ) || (matches!(ext, "toml" | "json" | "yaml" | "yml") && fname != "Cargo.toml");

        if is_key {
            index_file(conn, path, project_id, result)?;
        }
    }
    Ok(())
}

fn cleanup_deleted_files(
    conn: &rusqlite::Connection,
    project_path: &str,
    result: &mut IndexResult,
) -> Result<()> {
    let file_nodes = graph::get_nodes_by_type(conn, &NodeType::File)?;
    for node in file_nodes {
        if let Some(path) = node.data.get("path").and_then(|p| p.as_str()) {
            if path.starts_with(project_path) && !Path::new(path).exists() {
                graph::delete_node(conn, node.id)?;
                result.nodes_removed += 1;
            }
        }
    }
    Ok(())
}

fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}
