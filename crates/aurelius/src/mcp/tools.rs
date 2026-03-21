use serde_json::json;

pub fn tool_definitions() -> serde_json::Value {
    json!({
        "tools": [
            {
                "name": "memory_status",
                "description": "Full project snapshot for session start. Returns project structure, recent decisions with reasoning, open problems, solved problems, session history with summaries, and graph stats. Call this first in every new session.",
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
                "description": "Full-text search across the knowledge graph using FTS5. Use empty string or '*' to list recent nodes.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "FTS5 search query. Use empty string or '*' to list most recent nodes."
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
                "description": "Add a new knowledge node to the graph. Supports structured data and memory classification.",
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
                        },
                        "data": {
                            "type": "object",
                            "description": "Arbitrary JSON metadata (alternatives considered, related commits, context, etc.)"
                        },
                        "memory_kind": {
                            "type": "string",
                            "enum": ["semantic", "episodic"],
                            "description": "Memory classification: semantic (facts, concepts) or episodic (events, sessions). Default: semantic",
                            "default": "semantic"
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
                "name": "memory_update",
                "description": "Update an existing node's note and/or data. Use to enrich nodes with additional context, corrections, or structured metadata.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "UUID or label of the node to update"
                        },
                        "note": {
                            "type": "string",
                            "description": "New note text (replaces existing)"
                        },
                        "data": {
                            "type": "object",
                            "description": "New JSON metadata (replaces existing)"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "memory_session",
                "description": "Record a session summary with decisions made, problems solved, and next steps. Creates an episodic Session node linked to the project, plus Decision and Problem/Solution nodes. Call this at the end of a productive session to preserve cross-session context.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "summary": {
                            "type": "string",
                            "description": "Brief summary of what was accomplished this session"
                        },
                        "project": {
                            "type": "string",
                            "description": "Project name (used for linking and labeling)"
                        },
                        "decisions": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "List of decisions made and their reasoning"
                        },
                        "problems_solved": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "problem": { "type": "string" },
                                    "solution": { "type": "string" }
                                }
                            },
                            "description": "List of problem/solution pairs encountered"
                        },
                        "next_steps": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "What should be done next (carried forward to future sessions)"
                        },
                        "key_files": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Key files that were modified or are relevant"
                        }
                    },
                    "required": ["summary", "project"]
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
                "description": "Export the knowledge graph as JSON with pagination. Returns nodes and edges sorted by creation date (newest first).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "offset": {
                            "type": "integer",
                            "description": "Number of items to skip (default: 0)",
                            "default": 0
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum items to return (default: 50)",
                            "default": 50
                        }
                    },
                    "required": []
                }
            }
        ]
    })
}
