use serde_json::json;

pub fn tool_definitions() -> serde_json::Value {
    json!({
        "tools": [
            {
                "name": "memory_status",
                "description": "Full project snapshot for session start. Returns project structure, recent decisions with reasoning, open problems, solved problems, session history with summaries, and graph stats. Call this first in every new session.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": {
                            "type": "string",
                            "description": "Filter by project name (e.g. 'aurelius'). Shows only decisions, problems, sessions for this project."
                        }
                    },
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
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max nodes to return (default: 50). Seeds first, then by BFS depth.",
                            "default": 50
                        }
                    },
                    "required": ["topic"]
                }
            },
            {
                "name": "memory_search",
                "description": "Full-text search across the knowledge graph using FTS5. Use empty string or '*' to list recent nodes. Supports optional type filtering.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "FTS5 search query. Use empty string or '*' to list most recent nodes."
                        },
                        "type": {
                            "type": "string",
                            "description": "Filter by node type: decision, problem, solution, session, concept, project, crate, file, dependency"
                        },
                        "since": {
                            "type": "string",
                            "description": "Time filter: 'today', 'yesterday', '7d', '24h', or ISO 8601 datetime"
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
                "description": "Record a session summary with decisions made, problems solved, and next steps. Creates an episodic Session node linked to the project, plus Decision and Problem/Solution nodes. Optionally links to tasks. Returns active tasks for the project as a hint. Call this at the end of a productive session.",
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
                        },
                        "tasks": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "UUIDs or labels of tasks worked on during this session (creates related_to edges)"
                        }
                    },
                    "required": ["summary", "project"]
                }
            },
            {
                "name": "memory_recall",
                "description": "Smart recall: get everything the knowledge graph knows about a topic. Combines FTS search with BFS traversal, returns only knowledge nodes (decisions, problems, solutions, sessions, concepts) grouped by type. Skips structural noise (files, deps). Use this instead of separate search+context calls.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "topic": {
                            "type": "string",
                            "description": "Topic to recall knowledge about"
                        },
                        "depth": {
                            "type": "integer",
                            "description": "BFS traversal depth (default: 1, increase for broader recall)",
                            "default": 1
                        }
                    },
                    "required": ["topic"]
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
            },
            {
                "name": "memory_merge",
                "description": "Merge two duplicate or related nodes into one. Rewires all edges from 'source' onto 'target', removes self-loops and duplicate edges, optionally appends source's note to target, then deletes source. Use for deduplication of near-duplicates that memory_gc can't catch (different content_hash but same meaning).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "source": {
                            "type": "string",
                            "description": "UUID or label of the node to merge FROM (will be deleted)"
                        },
                        "target": {
                            "type": "string",
                            "description": "UUID or label of the node to merge INTO (survives)"
                        }
                    },
                    "required": ["source", "target"]
                }
            },
            {
                "name": "memory_gc",
                "description": "Garbage collection: removes duplicate edges, orphaned edges, and duplicate nodes (by content_hash). Run periodically to keep the graph clean.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "task_create",
                "description": "Create a well-structured task with title, description, acceptance criteria, and priority. Auto-links to project. Supports subtask hierarchy and blocking relations.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "Short, actionable task title"
                        },
                        "description": {
                            "type": "string",
                            "description": "Detailed description — what needs to be done and why"
                        },
                        "project": {
                            "type": "string",
                            "description": "Project name (auto-creates if missing). Default: 'unknown'"
                        },
                        "priority": {
                            "type": "string",
                            "enum": ["critical", "high", "medium", "low"],
                            "description": "Task priority (default: medium)",
                            "default": "medium"
                        },
                        "acceptance_criteria": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Definition of Done checklist — what must be true for this task to be complete"
                        },
                        "parent": {
                            "type": "string",
                            "description": "UUID or label of parent task (creates subtask_of edge)"
                        },
                        "blocks": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "UUIDs or labels of tasks that this task blocks"
                        }
                    },
                    "required": ["title"]
                }
            },
            {
                "name": "task_update",
                "description": "Update task status, priority, or acceptance criteria. Supports status transitions: backlog → active → done/blocked/cancelled. Auto-tracks started_at and completed_at timestamps.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "UUID or label of the task to update"
                        },
                        "status": {
                            "type": "string",
                            "enum": ["backlog", "active", "blocked", "done", "cancelled"],
                            "description": "New task status"
                        },
                        "priority": {
                            "type": "string",
                            "enum": ["critical", "high", "medium", "low"],
                            "description": "New priority"
                        },
                        "blocked_by": {
                            "type": "string",
                            "description": "Reason for blocking (auto-sets status to 'blocked')"
                        },
                        "note": {
                            "type": "string",
                            "description": "Update task description/notes"
                        },
                        "acceptance_criteria": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Replace acceptance criteria checklist"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "task_list",
                "description": "List tasks with filters by project, status, and priority. Sorted by priority (critical first), then by creation date. Shows work log count per task.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": {
                            "type": "string",
                            "description": "Filter by project name"
                        },
                        "status": {
                            "type": "string",
                            "description": "Filter by status (comma-separated: 'active,blocked')"
                        },
                        "priority": {
                            "type": "string",
                            "description": "Filter by priority level"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max results (default: 20)",
                            "default": 20
                        }
                    },
                    "required": []
                }
            },
            {
                "name": "task_log",
                "description": "Record work done on a task. Creates a WorkLog node linked to the task. Optionally records decisions made and problems solved during the work. Auto-activates backlog tasks on first log entry.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "task": {
                            "type": "string",
                            "description": "UUID or label of the task"
                        },
                        "text": {
                            "type": "string",
                            "description": "Description of work done"
                        },
                        "decisions": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Decisions made during this work"
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
                            "description": "Problem/solution pairs encountered"
                        }
                    },
                    "required": ["task", "text"]
                }
            },
            {
                "name": "task_stats",
                "description": "Analytics over tasks: counts by status and priority, completion rate, average/median time from active to done (hours), currently blocked count, oldest active task age, and tasks closed in the window. Filter by project and time window.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": {
                            "type": "string",
                            "description": "Filter by project name"
                        },
                        "since_days": {
                            "type": "integer",
                            "description": "Window size in days for 'done_in_window' metric (default: all time)"
                        }
                    },
                    "required": []
                }
            },
            {
                "name": "task_view",
                "description": "Full task overview with its entire knowledge branch: work logs (as timeline), decisions, problems, solutions, and subtasks. Shows acceptance criteria and current status.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "UUID or label of the task"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "search_web",
                "description": "Search the web via Brave Search API. Results are cached locally in SQLite — repeat queries don't burn API quota. Optionally saves results to the knowledge graph for future recall.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query"
                        },
                        "count": {
                            "type": "integer",
                            "description": "Number of results (default: 5, max: 20)",
                            "default": 5
                        },
                        "cache_days": {
                            "type": "integer",
                            "description": "How many days to cache results (default: 7)",
                            "default": 7
                        },
                        "save_to_graph": {
                            "type": "boolean",
                            "description": "Save results as a concept node in the knowledge graph (default: false)",
                            "default": false
                        }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "search_recall",
                "description": "Search through previously cached web search results via FTS. Use this to find information from past searches without hitting the API again.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "FTS query to search through cached results"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum results (default: 10)",
                            "default": 10
                        }
                    },
                    "required": ["query"]
                }
            }
        ]
    })
}
