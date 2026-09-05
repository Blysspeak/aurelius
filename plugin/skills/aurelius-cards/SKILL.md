---
name: aurelius-cards
description: Reusable how-to cards (au CLI reference, agent checkpoints, workflow orders) live in aurelius memory, not in this plugin. Load when a task needs one of them - the index arrives at SessionStart, the body comes from skill_get.
---

The cards are stored in the aurelius knowledge graph and served by the MCP server this plugin
registers. Do not look for their text here.

1. The SessionStart hook already printed the index: one line per card, name plus trigger.
2. Fetch a body by name: `mcp__aurelius__skill_get(name: "<card-name>")`.
3. No index in context (hook failed or was disabled): `mcp__aurelius__skill_list()`.
4. Working out a repeatable procedure worth keeping: `mcp__aurelius__skill_save(...)` - it lands
   next to the others and shows up in the next session's index.
