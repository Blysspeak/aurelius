use serde_json::json;

pub fn tool_definitions() -> serde_json::Value {
    json!({
        "tools": [
            {
                "name": "memory_status",
                "description": "Full project snapshot for session start. Returns project structure, recent decisions, open problems, activity summary, and TimeForged sessions.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "memory_context",
                "description": "Get contextual knowledge graph around a topic using BFS traversal from FTS seed nodes.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "topic": {
                            "type": "string",
                            "description": "Topic to search for and expand context around"
                        },
                        "depth": {
                            "type": "integer",
                            "description": "BFS traversal depth (default: 2)",
                            "default": 2
                        }
                    },
                    "required": ["topic"]
                }
            },
            {
                "name": "memory_search",
                "description": "Full-text search across the knowledge graph using FTS5.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "FTS5 search query"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum results (default: 20)",
                            "default": 20
                        }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "memory_add",
                "description": "Add a new knowledge node to the graph.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "label": {
                            "type": "string",
                            "description": "Short label for the node"
                        },
                        "type": {
                            "type": "string",
                            "description": "Node type: project, decision, concept, problem, solution, person, dependency, server, file, module, crate, config, session, language",
                            "default": "concept"
                        },
                        "note": {
                            "type": "string",
                            "description": "Detailed note/description"
                        },
                        "source": {
                            "type": "string",
                            "description": "Source of this knowledge (default: mcp)",
                            "default": "mcp"
                        }
                    },
                    "required": ["label"]
                }
            },
            {
                "name": "memory_relate",
                "description": "Create a typed edge between two nodes.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "from": {
                            "type": "string",
                            "description": "Label or ID of the source node"
                        },
                        "to": {
                            "type": "string",
                            "description": "Label or ID of the target node"
                        },
                        "relation": {
                            "type": "string",
                            "description": "Relation type: uses, depends_on, solves, caused_by, inspired_by, conflicts_with, supersedes, belongs_to, related_to, learned_from, contains, imports, exports, implements, configures, tracked_by"
                        },
                        "weight": {
                            "type": "number",
                            "description": "Edge weight (default: 1.0)",
                            "default": 1.0
                        }
                    },
                    "required": ["from", "to", "relation"]
                }
            },
            {
                "name": "memory_index",
                "description": "Index a project directory into the knowledge graph. Parses Cargo.toml, discovers crates, files, and dependencies.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute path to the project root"
                        }
                    },
                    "required": ["path"]
                }
            },
            {
                "name": "memory_forget",
                "description": "Delete a node from the knowledge graph by ID.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "UUID of the node to delete"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "memory_dump",
                "description": "Export the full knowledge graph as JSON (all nodes and edges).",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }
        ]
    })
}
