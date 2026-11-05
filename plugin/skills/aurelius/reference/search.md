# Search

Pick the lookup by what you know. Run `au search`, `au context`, `au recall`, `au journal` from any directory; they read the same database as the MCP tools.

## `au search "q"` (MCP `memory_search`): find by words or meaning

    au search "myapp backup"
    au search "myapp backup" --limit 10 --type decision,problem --json

- Hybrid: full-text over label and note (with a stemmed index, so another case or number of a word matches: "backup" finds "backups") plus BGE-M3 vectors, fused by RRF and, when a reranker is available, re-ordered. 5 hits by default; a larger `--limit` is honoured as asked.
- Global: there is no `-p/--project` flag, results span every project. Read the `[project]` prefix that task and session labels carry; notes written with `-p` keep their own label.
- `--type` keeps only those node types (repeatable or a comma list; a typo is refused, not answered with an empty list). `--json` adds `origin` per hit (`fts`, `dense` or `both`), `score`, `unmatched_terms` and `vector_notice`.
- No daemon, or no model files: the answer comes from full-text alone and says so (`vector_notice`, also printed in the plain output). Not a failure.
- An empty result is not proof the fact is missing. It means this exact string does not occur; the output lists which terms matched nothing. Retry shorter, or end a word with a star (`backu*`).
- Over MCP: `query` of `*` or empty lists the most recent nodes; `since` takes `today`, `yesterday`, `7d`, `24h` or an ISO date; a query shaped like a subject key (`myapp:db:pool-size`) puts the node with exactly that subject first.

## `au recall` (MCP `memory_search` with `id`): read one record, no fuzziness

    au recall myapp:db:pool-size           # a UUID or the exact subject
    au recall --prefix myapp:db            # every live subject starting with this facet

A miss is a clear "no node with subject ...", never a neighbour.

## `au context "myapp" -d 1` (MCP `memory_context`): graph neighbourhood

Walks edges out from the node whose label matches TOPIC (a project name, a claim label, anything in the graph). The CLI default depth is 2, MCP `memory_context` defaults to 1. Depth 2 and up walks through the project hub and out onto everything that shares the project, which is far too wide on a big project: start at `-d 1` and raise it only if the first pass misses. TOPIC is a label match across the whole graph, no project filter.

## `au journal --session <ID>`: what one run wrote

    SID="myapp-2026-11-05-a"
    au note --project myapp --session "$SID" --type decision --confidence measured \
      --evidence "curl -s localhost:8080/health" \
      --claim "Health endpoint answers 200 after a restart" "Verified after the worker was restarted" --json
    au session -p myapp --session "$SID" "Wired the health check into the deploy script" --json
    au journal --session "$SID"

`--session` is mandatory (or `AURELIUS_SESSION_ID`): there is no default "show me the last run", and omitting it exits 1. Use it at the end of a session to see exactly what this run wrote, since search and context return everything ever written, mixed with every other run.

## Trap

`--confidence measured` with no `--evidence` is refused (exit 1), not silently downgraded: "something measured without the command that measured it is inferred". Pair `measured` with a real command, or write `--confidence inferred`.
