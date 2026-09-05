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
                            "description": "BFS traversal depth (default: 1). Depth 2+ walks through project hub nodes and back out onto every unrelated task/decision that shares the project — expect a much larger, noisier result.",
                            "default": 1
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max nodes to return (default: 50). Seeds first, then by BFS depth. Response's `truncation` field says how many were hidden.",
                            "default": 50
                        }
                    },
                    "required": ["topic"]
                }
            },
            {
                "name": "memory_path",
                "description": "Directed step ladder over next_step/prerequisite edges, not a neighbourhood: shortest path between two nodes (from+to), or every node that transitively leads to one target (before), earliest first. Selectors resolve as UUID, then exact subject, then exact label. A missing path comes back as {error:...} in a normal result, not a tool error.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "from": {
                            "type": "string",
                            "description": "Start node selector. Required together with 'to'; mutually exclusive with 'before'."
                        },
                        "to": {
                            "type": "string",
                            "description": "End node selector. Required together with 'from'; mutually exclusive with 'before'."
                        },
                        "before": {
                            "type": "string",
                            "description": "Target node selector. Returns every ancestor instead of a from/to path; mutually exclusive with 'from'/'to'."
                        },
                        "max_depth": {
                            "type": "integer",
                            "description": "Walk depth cap (default: 50)",
                            "default": 50
                        }
                    },
                    "required": []
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
                "description": "Add a new knowledge node to the graph. Supports structured data and memory classification. Pass 'project' so the node is linked to that project — without a link (or a '[project]' label prefix) the node is invisible to memory_status(project=…) and to the snapshot, and the response carries an 'attachment_warning' saying so.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "label": {
                            "type": "string",
                            "description": "Short label for the node"
                        },
                        "project": {
                            "type": "string",
                            "description": "Project this node belongs to. Creates the belongs_to edge for you (and the project node if missing). Omit only for genuinely global knowledge."
                        },
                        "type": {
                            "type": "string",
                            "enum": aurelius_core::models::NodeType::KNOWN,
                            "description": "Node type. A typo used to become a custom type silently — a node no query filters on.",
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
                        },
                        "session_id": {
                            "type": "string",
                            "description": "The run that wrote this. Without it nothing distinguishes this record from one written yesterday, and 'everything written in this session' cannot be selected at all. Readable back with `au journal --session <id>`."
                        },
                        "confidence": {
                            "type": "string",
                            "enum": ["measured", "inferred", "reported", "unverified"],
                            "description": "Where this came from. REQUIRED — a false claim otherwise lands exactly like a measured one. measured: obtained by a command/query, which must be quoted verbatim in 'evidence'. inferred: derived from something measured, not itself measured. reported: told by a human or docs, unchecked. unverified: origin not named. Anything but 'measured' is marked as such on the way out."
                        },
                        "evidence": {
                            "type": "string",
                            "description": "The command or query VERBATIM — what produced this. Required when confidence is 'measured': a measurement without the command that made it is an inference."
                        },
                        "measured_at": {
                            "type": "string",
                            "description": "RFC 3339 timestamp of the measurement. Defaults to now for 'measured'. Pass it explicitly when recording something measured earlier."
                        },
                        "claim": {
                            "type": "string",
                            "description": "The assertion in one or two lines — returned WHOLE, never clipped mid-word. Max 240 chars; the long reasoning belongs in 'note', which is returned on demand."
                        },
                        "volatility": {
                            "type": "string",
                            "enum": ["immutable", "slow", "volatile"],
                            "description": "How fast this stops being true. immutable: never (a function address, a commit sha). slow: rarely and visibly (a DB schema). volatile: quietly and at any moment (a value in .env, a process state). A stale fact is handed back with 'older than N days — re-check with …'. Omit when you do not know; a wrong default would be the same silent lie this field exists to prevent."
                        },
                        "verify_with": {
                            "type": "string",
                            "description": "Command that re-checks this claim. Without it a staleness note reports trouble without saying how to close it."
                        },
                        "subject": {
                            "type": "string",
                            "description": "Identity of what is being asserted, e.g. 'xhub:.env:REFUND_REQUESTS_ENABLED'. Two facts sharing a subject cannot both be true, so a second one is refused until you say how to resolve it — see 'resolution'."
                        },
                        "resolution": {
                            "type": "string",
                            "enum": ["supersede", "refine", "coexist"],
                            "description": "How this relates to the existing fact about the same 'subject'. supersede: the old one is no longer true (creates a supersedes edge). refine: the old one stays true, this makes it more precise. coexist: both hold — say so deliberately."
                        }
                    },
                    "required": ["label", "confidence"]
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
                            // Список берётся из словаря ядра, а не переписан
                            // руками: копия уже успела отстать на две связи.
                            "description": format!("Relation type: {}", aurelius_core::models::Relation::KNOWN.join(", ")),
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
                "description": "Record a session summary with decisions made, problems solved, and next steps. Creates an episodic Session node linked to the project, plus Decision and Problem/Solution nodes. Optionally links to tasks. Returns active tasks for the project as a hint. Call this at the end of a productive session. Accepts the same provenance fields as memory_add (confidence, evidence, subject, volatility, claim, measured_at, verify_with), parsed the same way — they land on the session node; the decisions/problems/solutions it spawns inherit confidence/evidence but never subject or claim. resolution is not supported here: to supersede an existing fact by subject, use memory_add with resolution.",
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
                        "session_id": {
                            "type": "string",
                            "description": "The run that wrote this record. Stamps the session node and everything it spawns, so `au journal --session <id>` can list them back."
                        },
                        "tasks": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "UUIDs or labels of tasks worked on during this session (creates related_to edges)"
                        },
                        "confidence": {
                            "type": "string",
                            "enum": ["measured", "inferred", "reported", "unverified"],
                            "description": "Where this came from. REQUIRED — a false claim otherwise lands exactly like a measured one. measured: obtained by a command/query, which must be quoted verbatim in 'evidence'. inferred: derived from something measured, not itself measured. reported: told by a human or docs, unchecked. unverified: origin not named. Anything but 'measured' is marked as such on the way out."
                        },
                        "evidence": {
                            "type": "string",
                            "description": "The command or query VERBATIM — what produced this. Required when confidence is 'measured': a measurement without the command that made it is an inference."
                        },
                        "measured_at": {
                            "type": "string",
                            "description": "RFC 3339 timestamp of the measurement. Defaults to now for 'measured'. Pass it explicitly when recording something measured earlier."
                        },
                        "claim": {
                            "type": "string",
                            "description": "The assertion in one or two lines — returned WHOLE, never clipped mid-word. Max 240 chars; the long reasoning belongs in 'note', which is returned on demand."
                        },
                        "volatility": {
                            "type": "string",
                            "enum": ["immutable", "slow", "volatile"],
                            "description": "How fast this stops being true. immutable: never (a function address, a commit sha). slow: rarely and visibly (a DB schema). volatile: quietly and at any moment (a value in .env, a process state). A stale fact is handed back with 'older than N days — re-check with …'. Omit when you do not know; a wrong default would be the same silent lie this field exists to prevent."
                        },
                        "verify_with": {
                            "type": "string",
                            "description": "Command that re-checks this claim. Without it a staleness note reports trouble without saying how to close it."
                        },
                        "subject": {
                            "type": "string",
                            "description": "Identity of what is being asserted, e.g. 'xhub:.env:REFUND_REQUESTS_ENABLED'. Two facts sharing a subject cannot both be true, so a second one is refused until you say how to resolve it — see 'resolution'."
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
                "name": "memory_snapshot",
                "description": "Seven-layer frozen memory snapshot as compact Markdown (~1.5K tokens): 1 owner facts (user_fact), 2 active tasks + open problems, 3 recent sessions, 4 decisions/concepts, 5 skills, 6 archive pointers, 7 project digest. Built for direct context injection at session start (the SessionStart hook calls `au snapshot --hook`). Read-only and instant. Prefer this over memory_status when you need orientation, not raw data.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": {
                            "type": "string",
                            "description": "Project scope (label prefix). Omit for global snapshot."
                        },
                        "consolidate": {
                            "type": "boolean",
                            "description": "Rebuild the project digest (layer 7) before building the snapshot"
                        }
                    },
                    "required": []
                }
            },
            {
                "name": "memory_consolidate",
                "description": "Rebuild the project's Digest node (memory layer 7): distills next_steps from recent sessions plus unsolved problems into one compact note. Idempotent — one digest per project, overwritten in place. Run at session end or when the digest looks stale.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": {
                            "type": "string",
                            "description": "Project name"
                        }
                    },
                    "required": ["project"]
                }
            },
            {
                "name": "task_create",
                "description": "Create a well-structured task with title, description, acceptance criteria, and priority. Auto-links to project. Supports subtask hierarchy and blocking relations. Accepts the same provenance fields as memory_add (confidence, evidence, subject, volatility, claim, measured_at, verify_with), parsed the same way — they land on the task node. resolution is not supported here: to supersede an existing fact by subject, use memory_add with resolution.",
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
                        },
                        "confidence": {
                            "type": "string",
                            "enum": ["measured", "inferred", "reported", "unverified"],
                            "description": "Where this came from. REQUIRED — a false claim otherwise lands exactly like a measured one. measured: obtained by a command/query, which must be quoted verbatim in 'evidence'. inferred: derived from something measured, not itself measured. reported: told by a human or docs, unchecked. unverified: origin not named. Anything but 'measured' is marked as such on the way out."
                        },
                        "evidence": {
                            "type": "string",
                            "description": "The command or query VERBATIM — what produced this. Required when confidence is 'measured': a measurement without the command that made it is an inference."
                        },
                        "measured_at": {
                            "type": "string",
                            "description": "RFC 3339 timestamp of the measurement. Defaults to now for 'measured'. Pass it explicitly when recording something measured earlier."
                        },
                        "claim": {
                            "type": "string",
                            "description": "The assertion in one or two lines — returned WHOLE, never clipped mid-word. Max 240 chars; the long reasoning belongs in 'note', which is returned on demand."
                        },
                        "volatility": {
                            "type": "string",
                            "enum": ["immutable", "slow", "volatile"],
                            "description": "How fast this stops being true. immutable: never (a function address, a commit sha). slow: rarely and visibly (a DB schema). volatile: quietly and at any moment (a value in .env, a process state). A stale fact is handed back with 'older than N days — re-check with …'. Omit when you do not know; a wrong default would be the same silent lie this field exists to prevent."
                        },
                        "verify_with": {
                            "type": "string",
                            "description": "Command that re-checks this claim. Without it a staleness note reports trouble without saying how to close it."
                        },
                        "subject": {
                            "type": "string",
                            "description": "Identity of what is being asserted, e.g. 'xhub:.env:REFUND_REQUESTS_ENABLED'. Two facts sharing a subject cannot both be true, so a second one is refused until you say how to resolve it — see 'resolution'."
                        }
                    },
                    "required": ["title"]
                }
            },
            {
                "name": "task_update",
                "description": "Update task status, priority, or acceptance criteria. Supports status transitions: backlog → active → done/blocked/cancelled. Transitioning to 'active' stamps activated_at and evicts any other active task in the same project back to backlog (at most one active task per project) — same rule as `au task activate`, not a separate copy of it. Transitioning to 'done' stamps closed_at and builds the resolution (how the task got solved) the same way the CLI does: commit is read from the current git HEAD unless given explicitly, files come from edits traced since activation; optional commit/pull_request/unconfirmed only refine that auto-collected resolution, they don't replace it. Also auto-tracks legacy started_at/completed_at timestamps for older readers. Accepts the same provenance fields as memory_add (confidence, evidence, subject, volatility, claim, measured_at, verify_with), parsed the same way — a task's confidence can change after a measurement, and this is how that lands on the task node. resolution is not supported here: to supersede an existing fact by subject, use memory_add with resolution.",
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
                        },
                        "commit": {
                            "type": "string",
                            "description": "Only with status='done'. Commit that resolves the task — refines the resolution; if omitted, the commit is read from the repo's current HEAD automatically"
                        },
                        "pull_request": {
                            "type": "string",
                            "description": "Only with status='done'. Pull request that resolves the task — refines the auto-collected resolution"
                        },
                        "unconfirmed": {
                            "type": "boolean",
                            "description": "Only with status='done'. Force-mark the resolution as unconfirmed even if a commit/PR/edited files were found (default: unconfirmed only when nothing was found)"
                        },
                        "confidence": {
                            "type": "string",
                            "enum": ["measured", "inferred", "reported", "unverified"],
                            "description": "Where this came from. REQUIRED — a false claim otherwise lands exactly like a measured one. measured: obtained by a command/query, which must be quoted verbatim in 'evidence'. inferred: derived from something measured, not itself measured. reported: told by a human or docs, unchecked. unverified: origin not named. Anything but 'measured' is marked as such on the way out."
                        },
                        "evidence": {
                            "type": "string",
                            "description": "The command or query VERBATIM — what produced this. Required when confidence is 'measured': a measurement without the command that made it is an inference."
                        },
                        "measured_at": {
                            "type": "string",
                            "description": "RFC 3339 timestamp of the measurement. Defaults to now for 'measured'. Pass it explicitly when recording something measured earlier."
                        },
                        "claim": {
                            "type": "string",
                            "description": "The assertion in one or two lines — returned WHOLE, never clipped mid-word. Max 240 chars; the long reasoning belongs in 'note', which is returned on demand."
                        },
                        "volatility": {
                            "type": "string",
                            "enum": ["immutable", "slow", "volatile"],
                            "description": "How fast this stops being true. immutable: never (a function address, a commit sha). slow: rarely and visibly (a DB schema). volatile: quietly and at any moment (a value in .env, a process state). A stale fact is handed back with 'older than N days — re-check with …'. Omit when you do not know; a wrong default would be the same silent lie this field exists to prevent."
                        },
                        "verify_with": {
                            "type": "string",
                            "description": "Command that re-checks this claim. Without it a staleness note reports trouble without saying how to close it."
                        },
                        "subject": {
                            "type": "string",
                            "description": "Identity of what is being asserted, e.g. 'xhub:.env:REFUND_REQUESTS_ENABLED'. Two facts sharing a subject cannot both be true, so a second one is refused until you say how to resolve it — see 'resolution'."
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "task_list",
                "description": "List tasks with filters by project, status, and priority. Sorted by priority (critical first), then by creation date. Shows work log count per task, plus each task's activated_at/closed_at timestamps, resolution (how it was solved: commit, PR, files, confirmed), evidence (command runs recorded against it), and the derived 'ripe' flag — true when an active task has passing evidence newer than its last edit and is ready to present for closing.",
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
                "description": "Record work done on a task. Creates a WorkLog node linked to the task. Optionally records decisions made and problems solved during the work. Auto-activates backlog tasks on first log entry. Accepts the same provenance fields as memory_add (confidence, evidence, subject, volatility, claim, measured_at, verify_with), parsed the same way — they land on the WorkLog node; the decisions/problems/solutions it spawns inherit confidence/evidence but never subject or claim. resolution is not supported here: to supersede an existing fact by subject, use memory_add with resolution.",
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
                        },
                        "confidence": {
                            "type": "string",
                            "enum": ["measured", "inferred", "reported", "unverified"],
                            "description": "Where this came from. REQUIRED — a false claim otherwise lands exactly like a measured one. measured: obtained by a command/query, which must be quoted verbatim in 'evidence'. inferred: derived from something measured, not itself measured. reported: told by a human or docs, unchecked. unverified: origin not named. Anything but 'measured' is marked as such on the way out."
                        },
                        "evidence": {
                            "type": "string",
                            "description": "The command or query VERBATIM — what produced this. Required when confidence is 'measured': a measurement without the command that made it is an inference."
                        },
                        "measured_at": {
                            "type": "string",
                            "description": "RFC 3339 timestamp of the measurement. Defaults to now for 'measured'. Pass it explicitly when recording something measured earlier."
                        },
                        "claim": {
                            "type": "string",
                            "description": "The assertion in one or two lines — returned WHOLE, never clipped mid-word. Max 240 chars; the long reasoning belongs in 'note', which is returned on demand."
                        },
                        "volatility": {
                            "type": "string",
                            "enum": ["immutable", "slow", "volatile"],
                            "description": "How fast this stops being true. immutable: never (a function address, a commit sha). slow: rarely and visibly (a DB schema). volatile: quietly and at any moment (a value in .env, a process state). A stale fact is handed back with 'older than N days — re-check with …'. Omit when you do not know; a wrong default would be the same silent lie this field exists to prevent."
                        },
                        "verify_with": {
                            "type": "string",
                            "description": "Command that re-checks this claim. Without it a staleness note reports trouble without saying how to close it."
                        },
                        "subject": {
                            "type": "string",
                            "description": "Identity of what is being asserted, e.g. 'xhub:.env:REFUND_REQUESTS_ENABLED'. Two facts sharing a subject cannot both be true, so a second one is refused until you say how to resolve it — see 'resolution'."
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
                "description": "Task overview: the task itself (never truncated — status, priority, acceptance criteria, activated_at/closed_at, resolution, evidence, the derived 'ripe' flag) plus its own knowledge branch (work logs as a timeline, decisions, problems, solutions, direct subtasks). By default the branch is capped at 5 most-recent items per category with notes clipped to 300 chars at a word boundary; a 'truncation' block in the response always reports exactly how many nodes of each type were left out and how to get them. Pass full=true to get the whole branch untruncated, or limit=N to change the per-category cap.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "UUID or label of the task"
                        },
                        "full": {
                            "type": "boolean",
                            "description": "Skip truncation entirely: every branch node, notes untrimmed (default: false)"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max nodes shown per category (timeline/decisions/problems/solutions/subtasks) when full is not set (default: 5)"
                        }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "task_ripe",
                "description": "List tasks ready to close: active tasks with a passing (exit-0) evidence run newer than their last edit, each with the basis for the claim — which evidence run, when, and which files were touched since the task was taken active. This is the same computation `au task ripe` runs on the CLI, exposed here because closing a task via MCP is `task_update`, and nothing else on this surface could tell you what has ripened. Declining a proposal is CLI-only (`au task ripe --decline <id>`) — not exposed here.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": {
                            "type": "string",
                            "description": "Filter by project name. Omit to check every project."
                        }
                    },
                    "required": []
                }
            },
            {
                "name": "secret_list",
                "description": "List where each project's secrets live — name, purpose, and location (env var / file path / password manager reference) — never the value itself. Aurelius refuses to store secret values (`au secret add` rejects anything that looks like one); this only reads coordinates already recorded that way. Coordinates are intentionally excluded from memory_snapshot and every other automatic dump, so this is the only MCP path to them. Adding or removing a coordinate stays CLI-only (`au secret add`/`rm`) — recording one is a deliberate human act, and an MCP write path risks a model writing the actual secret value into 'location' by mistake, caught only heuristically.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "project": {
                            "type": "string",
                            "description": "Filter by project name. Omit to list every project's coordinates."
                        }
                    },
                    "required": []
                }
            },
            {
                "name": "search_web",
                "description": "Search the web via Brave Search API or Perplexity Search API, selected by 'provider'. Results are cached locally in SQLite, scoped per provider — repeat queries don't burn API quota. Optionally saves results to the knowledge graph for future recall.",
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
                        },
                        "provider": {
                            "type": "string",
                            "enum": ["brave", "perplexity"],
                            "description": "Which search backend to use (default: brave). 'brave' calls the Brave Search API (src/search/brave.rs, key via BRAVE_API_KEY or ~/.config/aurelius/brave.key); 'perplexity' calls the Perplexity Search API (src/search/perplexity.rs, key via PERPLEXITY_API_KEY or ~/.config/aurelius/perplexity.key).",
                            "default": "brave"
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
            },
            {
                "name": "doc_convert",
                "description": "Convert a document to GitHub-Flavored Markdown and cache it. Handles Word (doc/docx/docm), PowerPoint (ppt/pptx/pps/pot), Excel (xls/xlsx/xlsm/xlsb), OpenDocument (odt/ods/odp), RTF, EPUB, CSV, PDF, HTML, and plain text or source files. Runs locally — no network, no API key. Point it at a directory to convert everything in it. Not supported: audio/video transcription and OCR of images or scanned pages.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute path to a file, or to a directory to convert in bulk"
                        },
                        "recursive": {
                            "type": "boolean",
                            "description": "Directory mode: descend into subdirectories. Respects .gitignore either way.",
                            "default": false
                        },
                        "max_files": {
                            "type": "integer",
                            "description": "Directory mode: cap on files converted (default: 200)",
                            "default": 200
                        },
                        "max_inline_chars": {
                            "type": "integer",
                            "description": "Markdown at or below this length is returned in full; longer output is written to a .md file and returned as outline + preview + path, readable via doc_read (default: 40000)",
                            "default": 40000
                        },
                        "project": {
                            "type": "string",
                            "description": "Project this document belongs to. Used when save_to_graph is set."
                        },
                        "save_to_graph": {
                            "type": "boolean",
                            "description": "Also create a 'document' node linked to the project. Metadata and an excerpt only — the full text stays in the cache.",
                            "default": false
                        },
                        "force": {
                            "type": "boolean",
                            "description": "Re-convert even if this exact content was converted before",
                            "default": false
                        }
                    },
                    "required": ["path"]
                }
            },
            {
                "name": "doc_read",
                "description": "Read a slice of an already-converted document from the cache. Use after doc_convert reports a document too large to inline. Offsets and limits are in characters.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "ref": {
                            "type": "string",
                            "description": "The sha256 doc_convert returned, or the source file path"
                        },
                        "offset": {
                            "type": "integer",
                            "description": "Character offset to start at (default: 0)",
                            "default": 0
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Characters to return (default: 40000)",
                            "default": 40000
                        }
                    },
                    "required": ["ref"]
                }
            },
            {
                "name": "doc_recall",
                "description": "Full-text search across every document ever converted, even if the original file is gone. Returns matching snippets with the reference needed to read the full text via doc_read.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "FTS5 query over document text and file names"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max results (default: 10)",
                            "default": 10
                        }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "skill_list",
                "description": "Cheap skill index — returns only name + trigger + tags (never the body) for every stored skill card. This is the progressive-disclosure index: scan it to see what reusable how-to knowledge exists, then call skill_get to read the full body. Optional FTS query / tag filter.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Optional FTS filter over skill name/trigger. Omit to list all skills."
                        },
                        "tag": {
                            "type": "string",
                            "description": "Optional: only skills carrying this tag"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max skills to return (default: 200)",
                            "default": 200
                        }
                    },
                    "required": []
                }
            },
            {
                "name": "skill_get",
                "description": "Fetch one skill card's full markdown body by name. Use after skill_list (or memory_search) surfaces a relevant skill. Bumps the skill's access_count.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Exact skill name (the label shown by skill_list)"
                        }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "skill_save",
                "description": "Create or update a skill card (upsert by name). A skill is reusable procedural knowledge — a 'how to do X' card. The trigger is a short 'when to apply this' line (FTS-indexed, so it's discoverable); the body is the full markdown instructions (stored verbatim, not keyword-indexed). Save a skill whenever you work out a repeatable procedure worth reusing across sessions.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Unique skill name, kebab-case (e.g. 'patch-shaiya-pe'). Used as the lookup key."
                        },
                        "trigger": {
                            "type": "string",
                            "description": "When to apply this skill — one or two sentences. This is what search matches on."
                        },
                        "body": {
                            "type": "string",
                            "description": "Full markdown instructions: steps, commands, gotchas, examples."
                        },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional tags for grouping/filtering (e.g. ['shaiya', 'reverse'])"
                        }
                    },
                    "required": ["name", "trigger", "body"]
                }
            },
            {
                "name": "skill_remove",
                "description": "Delete a skill card by name. Use when a skill is obsolete or wrong.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Exact skill name to delete"
                        }
                    },
                    "required": ["name"]
                }
            }
        ]
    })
}
