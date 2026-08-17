# Changelog

## [Unreleased]

Seven fixes on one root: memory accepted a claim about the world without asking where
it came from, and stayed silent when it accepted only half of what it was handed.

### Added
- **A fact now carries its provenance, and `confidence` is required.** A guess landed in the graph looking exactly like a measurement, and six weeks later read back as truth — the project already pays for this rule in money, where a number is never spoken without the query that produced it and the time it was measured. `memory_add` and `au note` take `confidence` (`measured` | `inferred` | `reported` | `unverified`), `evidence` — the command or query verbatim — and `measured_at`. `measured` without `evidence` is refused rather than accepted: a measurement nobody can repeat is an inference. An absent `confidence` reads as `unverified`, never as "probably measured", and anything below `measured` is marked on the way out, so the reader sees the doubt without having to look for it
- **Volatility, because `semantic` / `episodic` never caught it.** "The .env says true" is neither an event nor an eternal truth: it holds until the first edit of that file, and leaves no trace in git. `volatility` (`immutable` | `slow`, 30 d | `volatile`, 1 d) plus `verify_with` mean that past its age the fact comes back with "старше N дн — перепроверь вот этим" attached, instead of presenting itself as current
- **Contradictions are refused at the door.** "Disabled" and "enabled" could sit in the graph side by side without a word of objection, and the `supersedes` edge between them got placed by hand — that is, from memory, when someone happened to remember. An optional `subject` (`xhub:.env:REFUND_REQUESTS_ENABLED`) names what is being asserted; a second fact about the same subject is refused, listing what is already on record, until `resolution` says `supersede`, `refine` or `coexist`. The resolution then becomes an edge, so the next reader sees how the two relate instead of guessing
- **`claim` — the assertion in one or two lines, returned whole.** Recall clipped by character count, so the startup snapshot cut every fact mid-word: the substance sat at the end of a long note and never survived the ellipsis. A `claim` (≤240 chars) is never clipped; the long reasoning stays in the note and comes out on demand
- **`au capture --hook` — catching a fact at the moment of discovery.** The session-end hook shouts "save, compaction is near", which means memory gets written from an already degraded context, from a recollection of what happened. This one fires the other way round: `psql`/`ssh`/`kubectl`/`curl` just returned data, the output is still on screen, and the hook offers to save it as a measured fact with that exact command already in `evidence`. It writes nothing itself — a command with output is not yet a fact worth keeping, and auto-saving every successful query would turn the graph into a dump. The recogniser is deliberately narrow: a hook that fires on everything is read for a day and ignored thereafter
- **The seven provenance fields exist on both doors and share one parser.** `au note --confidence …` and `memory_add(confidence=…)` cannot drift apart on what a measured fact is

### Fixed
- **An unknown parameter name is now an error instead of a silent skip — this is the expensive one.** `memory_session` was called twice with wrong parameter names; both times the answer was `created: true`, and the only sign of trouble was the string `[unknown]` inside a label. The decisions and the next steps were gone, and two orphan nodes were left hanging outside the project. Every MCP call is now checked against the same `inputSchema` served in `tools/list` — one source of truth, so the check cannot drift from the contract — and an unknown name is refused with a "did you mean" suggestion and the words "ничего не записано". All wrong names are listed at once: fixing a call one name per attempt is the same waste the silent skip was. Enum values are checked too, which closes a matching hole — `NodeType::parse` used to turn a typo into `Custom(…)`, giving a node a type no query looks for, while the CLI rejected the same string
- **The response now says what was stored and what was dropped.** A parameter with a correct name and an empty value looked delivered: `decisions: []` silently records nothing, and the caller found out only by opening the graph. `memory_add` and `memory_session` return `stored_fields` and `dropped_fields`, and the session response additionally counts `decisions_written` and `problems_written`
- **A failed probe now downgrades `confidence` to `unverified` by itself.** `probe_warnings` worked and reported honestly, but it was a string in a response, and a string is easy to ignore. A fact whose verification failed no longer presents itself as measured — the field does the work without being read

### Notes
- BREAKING for callers of `memory_add`: `confidence` is now required. The intent is exactly that — a fact whose origin nobody stated is a fact nobody can trust.
- Schema V14 adds one partial expression index for `subject`. Records written before this release simply carry no provenance: they read as `unverified`, which is what they always were.
- The MCP server is a long-lived process: restart the client before expecting the new parameters to be accepted.

---

## [v1.12.0] — 2026-08-16

### Added
- **`au session` — the record layer 4 of the snapshot reads, written without a model in the loop.** «Последние сессии» is assembled exclusively from `Session` nodes, and the only thing able to write one was `memory_session` over MCP. A mechanical hook could reach for `au note`, but a note is not a session: it landed in layer 5, among lasting facts, next to decisions that are meant to outlive the day. So the most important record of a session depended on whether the model remembered to call a tool — and what does not happen mechanically does not happen. The writing itself moved into the core (`graph::record_session`): the MCP handler and the CLI now call one function, and what is left in the handler is only what belongs to the transport — task linking, the active-tasks hint, the sync push. Accepts the same shape the tool takes (`summary`, `decisions`, `problems_solved`, `next_steps`, `key_files`), from arguments or from a single JSON on stdin, deduplicated by `sha256(project|summary)` so a hook that fires twice for one occasion leaves one record. An unknown key in that JSON is an error, not a shrug (117e6b9)
- **`au relate` — edges from mechanics.** `au note --json` returns the id of what it wrote, but there was nothing to attach it to: `memory_relate` lived only in MCP, so everything written mechanically settled into the graph without a single edge — a heap, not a graph. The relation vocabulary moved into the core alongside it (`Relation::KNOWN` / `parse_known`), because the hand-written copy in the tool description had already drifted, missing `subtask_of` and `blocks`. Hyphens are accepted next to underscores, `part-of` is a spelling of `belongs_to` rather than a new variant — two names for one relation would have split every project-scoped query — and `refines` is added as a real one, since "makes earlier knowledge more precise without replacing it" had no equivalent. Repeating a call returns the existing edge and `created: false`; `add_edge` inserts with `OR IGNORE` and used to hand back an id that was not in the database (117e6b9)
- **A record now carries the run that wrote it.** The journal could not tell sessions apart at all: `session_id` lived only in the labile recall window, and the nodes themselves carried nothing. A session-end hook therefore saw every record of the project and had no way to separate its own from yesterday's — "collect everything I wrote this run" was impossible mechanically, only by eyeballing timestamps. `au note --session`, `au session --session` and `memory_add`/`memory_session` (`session_id`) stamp it; `AURELIUS_SESSION_ID` is the fallback for hooks whose stdin is already occupied by the note text. `au journal --session <id>` reads it back, which is the other half of the feature — stamping without a way to select is the same as not stamping. An unknown run stays an *absent* key rather than an empty string, so the selection can never accidentally match records nobody marked
- **`au context` prints the edges themselves**, not merely how many there were (117e6b9)

### Fixed
- **A hyphen in a search query was being parsed as an operator, so half the names in this graph were unsearchable.** `memory_search("rust-clean-code")` answered "no such column: clean" and `"skills-store"` answered "no such column: store": FTS5 reads a `MATCH` string as an expression, where `-` is `NOT` and `:` selects a column. Almost every skill and half the projects are named that way, so the symptom read as a broken database rather than as query syntax. Every word is now wrapped as a phrase — inside a phrase the operators lose their power, and the tokenizer still splits `skills-store` into two tokens and finds them adjacent — while explicit `AND`/`OR`/`NOT`/`NEAR` and a trailing prefix star survive, because existing callers lean on them. Wired into graph search, `doc_recall` and the web-search cache
- **Traversal returned every edge twice.** An edge is visible from both of its endpoints, so BFS collected it again on the next hop: the "N edges" count in `au context` was wrong and `memory_context` duplicated edges in its JSON. Invisible until the edges themselves were printed (117e6b9)
- **The exit codes were inverted.** clap answered `2` for a typo in an argument, while everything else — including an unreachable database — collapsed into `1`. The contract is now `0` done, `1` bad call, `2` storage unreachable, classified by walking the anyhow chain; no call site needed touching, since `db::open` already returns a typed `DbError`. The `--hook` variants remain the deliberate exception: they never fail and stay silent (117e6b9)

### Notes
- Schema V13 adds one partial expression index for the run stamp. Records written before this release simply carry no run — they read as "unknown", never as a match.
- The MCP server is a long-lived process: restart the client before expecting `memory_search` to stop failing on hyphens.

---

## [v1.11.1] — 2026-08-15

### Fixed
- **A URL, a version number and a fragment of a path were all being probed as files.** Stage-2 probes extract verifiable claims from a node's text and run them against the file system, so a claim that fails is reported back in `probe_warnings`. A single release note — one containing a GitHub link and a path to a source file — produced three warnings about paths that were never claimed and never existed. Three separate false positives shared one regex. A URL matched the pattern twice over: inside `https://`, the fragment `s:/` reads as a Windows drive, and everything after the host reads as an absolute path; URLs are now stripped *before* extraction, because once a URL has been cut into pieces its pieces are indistinguishable from paths. `/Blysspeak/aurelius/releases/tag/v1.11` passed as a path whose extension was `.11` — an extension made only of digits is a version number, not a file. And `crates/aurelius-core/src/graph/search.rs` was matched from its interior slash onward, yielding `/aurelius-core/src/graph/search.rs`, an assertion the author never made; Rust's regex engine has no lookbehind and `\b` does not help here, since there *is* a word boundary before a slash, so the character preceding a match is now examined directly. A false probe is worse than a missing one: it fires on every write and trains the caller to stop reading warnings, which is the only thing warnings are for (c7e3f16)

---

## [v1.11.0] — 2026-08-15

### Added
- **`au snapshot --json` — the snapshot in a shape a program can rely on.** Hooks are ordinary processes: they cannot reach the MCP server, so the CLI is their only channel, and until now that channel spoke Markdown. A consumer parsing `## N · Heading` with a regex depends on the layout, which means the next change of layout breaks it as quietly as a closed channel does. The shape is now fixed: `{"project":…,"facts":[{"kind","text","at"}]}`, where `kind` names the source layer (`userfact`, `task`, `problem`, `obligation`, `session`, `decision`, `concept`, `skill`, `digest`). An empty `facts` with exit 0 means "nothing to say"; no output, or a non-zero exit, means "broken" — the two states a silent channel used to be indistinguishable between. Separate exit codes per state are deliberately not added: the shape already tells them apart. Fact text is returned whole, without the per-layer budget clipping the Markdown form applies, because the budget belongs to the consumer and a silently shortened fact reads exactly like a short one. Both forms are assembled from a single gathering pass, so they cannot drift apart unnoticed (f49ec85)
- **`memory_add` takes a `project` and says so when a node is left unattached.** A node linked to no project — neither by the `[project]` label prefix nor by an edge — is not returned by any project-scoped query, and `memory_add` still answered `"created": true`. A write nobody will ever find has no business looking like a success. Passing `project` now creates the `belongs_to` edge (and the project node, if it is missing); omitting it puts an `attachment_warning` in the response naming the consequence — the node will appear in neither `memory_status(project=…)` nor the snapshot. Types that are global by nature (`project`, `userfact`, `skill`) are exempt. The rule is stated in the tool schema, because a parameter an agent never reads about is a parameter that does not exist (183f7b2)

### Fixed
- **Project membership was a string convention, so half the graph was invisible to the project it belonged to.** A snapshot of a project with a full graph behind it returned only the two housekeeping layers — the node counter and the distillate — while `memory_status` for the same project returned nothing at all. Membership was read exclusively from the label: `label LIKE '[project]%'`, with `memory_status` additionally hunting for knowledge by full-text-searching the literal `"[project]"`. But `memory_add` stores a plain label and the link to the project is a separate edge created by `memory_relate` — an edge no query read. The documented way to record knowledge produced nodes unreachable by every project-scoped lookup, and the failure was silent in both directions: an empty section is indistinguishable from an empty subject. Four hand-rolled filters are replaced by one predicate, `project_scope_sql` — label **or** edge. Edge direction and relation type are deliberately not checked: `memory_relate` links node→project, the indexer links project→file, and the relation vocabulary is open, so a miss there would again mean losing knowledge quietly; false positives stay bounded by the caller's node-type filter. Two smaller defects of the same family are fixed alongside. `task_create` writes `status: "backlog"` while every query asked for `active,blocked`, so a freshly created task was invisible everywhere until someone activated it by hand — creating a task meant losing it; `OPEN_TASK_STATUSES` now covers all three. And the placeholder distillate, "Хвостов нет — чисто.", is dropped from both output forms: it carries zero information, spends layer budget, and made a brand-new project return a one-line body instead of an empty one (32db29e)

### Notes
- No schema change: V12 is still the current version, and the fix is entirely in how the graph is queried. Knowledge written before this release becomes visible as soon as the new binary runs — nothing needs re-importing.
- The MCP server is a long-lived process. After upgrading, restart the client (Claude Code) before expecting `memory_status` or `memory_add` to behave differently.
- The Markdown snapshot is unchanged in shape and remains the default; `--json` is additive and conflicts only with `--hook`, which prints its own SessionStart envelope.

---

## [v1.10.0] — 2026-08-12

### Added
- **A seven-layer memory snapshot, injected at session start.** `build_snapshot` renders a frozen Markdown slice of the graph under a hard character budget per layer (~4.5K in total), read-only and instant. `consolidate` distils a project into a single `Digest` node — the next steps recorded by recent sessions plus the problems still unsolved — idempotently, so running it twice changes nothing. Two node types anchor the ends of the range: `UserFact` for what is known about the owner (layer 1) and `Digest` for the distillate (layer 7). Exposed as the `memory_snapshot` and `memory_consolidate` MCP tools and as `au snapshot [--project] [--hook]`, wired to Claude Code's SessionStart hook, which injects the slice straight into the context. A failing hook is swallowed: memory has no right to break the start of a session (b3ed9d4, 61837ce)
- **Bit-i-Delo, stage 1 — an append-only journal of what the agent actually did.** Schema V9 adds `act_trace`, a write-ahead log of actions mirrored into FTS5, alongside the `probes`, `pathways`, `labile_window`, `trace_attribution` and `corrections` tables the later stages consume. `trace.rs` ingests a trace with a payload ceiling, SHA-256 hashes of the file state before and after, and a strict enum of kinds — an unknown kind is the caller's error, not a new typo in the journal. Append-only is enforced by database triggers and pinned by tests: the history is not edited after the fact. `au trace` takes a trace from the command line or, with `--hook`, from Claude Code's PostToolUse JSON on stdin. The architecture behind all seven stages is written up as spec `003-bit-i-delo` (809fa5f)
- **Stage 2 — claims checked against ground truth.** `probes.rs` extracts verifiable statements from a node's text deterministically — file paths and git SHAs — and executes them against reality: the file system and `git cat-file`. `memory_add` records the outcome and returns `probe_warnings`. Advisory for now: a failed probe warns but does not yet change what is stored (66e268c)
- **Stage 3 — a surprise gate, recall as a transaction, and an outcome judge that calls no model.** `codec.rs` scores new text by normalised compression distance against zstd dictionaries trained per scope, so restating what is already expected costs almost nothing and reads as almost no news. `window.rs` turns recall into a transaction: a query signature, labile windows, path locking, and corrections served first; `memory_search` accepts a `session_id` and opens a window. `differ.rs` is a pure `judge(traces) -> Verdict` over `reinforce` / `erode` / `fork` / `null` — the verdict is computed, not asked; the reconsolidator writes revisions into `node_version`, and an `erode` debits the path and mints a correction. Wired as `au judge [--hook]` on the Stop hook. Schema V10 adds `codec`, `delta` and `node_version` (3a44c5c)
- **Stage 4 — clearing, obligations and bankruptcy, closing the loop.** `ledger.rs` clears the session ledger: a yield bonus for windows that reinforced, a penalty for what was rendered and never used, and `node_value_bits` as the single currency ranking is done in. `bankrupt_and_absorb` is garbage collection by insolvency — a node that has earned nothing is absorbed by its strongest neighbour and hands over its edges, reversibly through `node_version`; `memory_gc` triggers it. `obligations.rs` adds the prospective contour: a promise is taken in when a commissive marker appears, deduplicated by the trace that produced it, and settled only by a later event sharing at least two significant words with it — one shared word is usually the project name and would settle the wrong debt. Tension grows with age and with how often that counterparty has broken promises before, and the snapshot gained a «Давление» (pressure) layer to show it. Schema V11 (b6cb6b7)

### Fixed
- **An obligation is born from speech, not from a shell command.** The pressure layer was showing the owner lines like «aurelius blyss force foreach item local path remove silentlycontinue». Two defects sat behind it. Intake was fed the raw payload of a trace, and `au trace -m "надо потом добить клиринг"` mentions a promise without being one; the marker was also matched as a substring, so `todo` was found inside `TodoWrite`. A speech gate now rejects text that is too long or too dense in punctuation, and the marker is matched on word boundaries — the gate is deliberately one-sided, since a missed obligation is cheaper than an invented one. Separately, what was displayed was `object_fp`, the alphabetically sorted bag of tokens used for deduplication and search, so even a correct extraction read as noise; the readable sentence is now stored beside the fingerprint and the snapshot shows that instead. Migration V12 adds the column and re-checks every obligation already on record against its original text in `act_trace`: what fails the gate should never have existed and is deleted (2df74f3)

### Notes
- Schema V9 through V12 all land here. Migrations run in a single transaction and are applied on first open.
- The snapshot now has eight sections; «Давление» sits third, between what is in progress and the recent sessions.
- The three Claude Code hooks close a loop: `au snapshot` on SessionStart feeds the context, `au trace` on PostToolUse records what happened, `au judge` on Stop settles it.

---

## [v1.9.0] — 2026-08-08

### Added
- **Documents into Markdown, converted locally.** Word, PowerPoint, Excel, OpenDocument, RTF, EPUB, CSV, PDF, HTML and plain text, via the `anydoc` and `htmd` crates — in-process, with no network call, no API key and no external binary. Three MCP tools (`doc_convert`, `doc_read`, `doc_recall`) plus `au doc convert` / `au doc recall`. Output over `max_inline_chars` spills to a `.md` file and returns an outline and preview instead, so a 200-page PDF cannot fill an agent's context in one call. Conversions are cached by the SHA-256 of the file contents and full-text indexed, which is the point: a contract read in July stays findable in September after the original file is gone. Audio, video and scanned images are refused by name with the reason — transcription and OCR need external services and are deliberately out of scope (8a022b2)
- **Sync — one graph across machines and people.** New `aurelius-sync-server` crate, the `au share` command family, and automatic push/pull at session boundaries. Every node and edge carries who created and last updated it; deletions propagate as tombstones rather than reappearing on the next pull; conflicting edits keep the losing version on the node instead of discarding it, surfaced by `au context --verbose`. Collaborator access is granted per project by an issued token and can be revoked (92872ba, f2cc833, 36c5382, d92ff92, d8f7bbc, 2f033b0)
- `au home use` / `current` / `reset` — a persisted active profile, so a chosen home survives the shell that set it (6e83cd8)
- `au share admin-set` — stores this machine's admin token per server, so `issue` and `revoke` no longer need `AURELIUS_SYNC_ADMIN_TOKEN` exported every session (3c99ff9)
- Identity falls back to `git config --global` when unset, so attribution works before anyone configures it (858af64)

### Fixed
- **A push could write outside its project.** The server now enforces the granted project scope on every pushed node and edge, instead of trusting the client's own labelling (91440a5)
- `aurelius-sync-server` binds to loopback only by default. Exposing it needs a deliberate choice, not an oversight (6a3f661)
- `au share issue --for` defaults to the local identity instead of failing when omitted (6ffe192)
- `au share admin-set` shows its `<server> <token>` usage in `--help` (a80f0d9)

### Documentation
- Spec-kit features `003-doc-to-markdown` and the project-sync set — specification, plan, research, data model, contracts, quickstart and task list (8a022b2, be9f6f3, 4bdcbd7, a18b124)

### Notes
- **Sync ships for the first time here.** It landed across PRs #5–#12 after the v1.8.0 tag, so this release is the first one that contains it.
- Schema V8 adds the document cache and its FTS mirror; V6 and V7, which sync introduced, are also first released here. Migrations run in a single transaction and are applied on first open.

---

## [v1.8.0] — 2026-07-29

### Added
- `au db check [PATH]` — the check now takes an optional path, so a snapshot can be verified. A snapshot is an ordinary database, so verifying one is the same command pointed at a different file (a22d373)
- The rolling backup hook verifies every snapshot it writes. An unverified backup is a guess. A snapshot that fails the check is renamed to `.FAILED-CHECK` — out of the `aurelius-*.db` pattern, so it can never be mistaken for a good backup nor counted by retention — and kept rather than deleted, because a bad snapshot is evidence worth reading (a22d373)

### Fixed
- `au db backup` reports a missing database instead of failing inside the snapshot call (a22d373)

---

## [v1.7.0] — 2026-07-29

### Fixed
- **A damaged database is refused instead of silently rewritten.** Every `db::open` now checks the file's own 100-byte header against its size and refuses an image whose header describes less than the file holds — the exact fingerprint of a file-level copy over a live WAL database. The error names the file, the finding, the two commands to run, and the rule that caused the damage, instead of the bare `database disk image is malformed` (d781fe2)
- **A failed read is no longer mistaken for an empty database.** `get_schema_version(...).unwrap_or(0)` turned `SQLITE_BUSY` and `SQLITE_CORRUPT` into "brand-new database" and re-ran the destructive migration chain over live data on every single invocation. Version reads now propagate their errors; zero means only that the `schema_version` table is absent (d781fe2)
- **Migrations are atomic.** The whole chain runs in one `BEGIN IMMEDIATE` transaction with the version re-read inside it, so a failure mid-`migrate_v4` can no longer leave the FTS index dropped, its triggers gone and the version advanced. Concurrent processes block instead of racing — 8 simultaneous opens of a fresh database used to fail with `UNIQUE constraint failed: schema_version.version` (d781fe2)
- **Concurrent access waits instead of failing.** Every connection sets `busy_timeout` before anything can take a lock — a hook spawns a writer on every file edit, and several MCP servers run at once, so contention is the norm rather than the exception (d781fe2)
- **WAL mode is verified, not assumed.** `PRAGMA journal_mode` reports a refused switch as a result row rather than an error, and `execute_batch` discarded that row — a connection could silently run in rollback-journal mode. The mode is now read back and checked, with bounded retries for the brief exclusive lock a fresh database needs (d781fe2)
- Databases written by a newer binary are refused instead of being written to under an older understanding of the schema (d781fe2)
- Failed edge writes propagate instead of being discarded at 20 call sites; `touch_node` and `ensure_indexed` stay best-effort — an access counter must never fail a read — but are logged rather than silently dropped (d781fe2)
- `migrate_v2` detects existing columns structurally via `pragma_table_info` instead of matching the English text of an error message (d781fe2)
- The database path had three divergent definitions that disagreed on their fallback; on a machine without a data directory the CLI and the MCP server would have used different files. Now resolved in one place (d781fe2)

### Added
- `au db backup [--out PATH]` — safe snapshot of a live database via SQLite's own `VACUUM INTO`, including data still sitting in an un-checkpointed `-wal`. **Copying `aurelius.db` with `cp`/`mv`/`rsync` while `au` or an MCP server is running is what corrupts it** — use this instead (d781fe2)
- `au db check [--full]` — read-only integrity report that never migrates and never writes a page. Exits non-zero when damaged, so it can gate a script or a hook (d781fe2)
- Skills subsystem — 4 MCP tools (`skill_save`, `skill_list`, `skill_get`, `skill_remove`), `au skills`, and session auto-injection via a SessionStart hook. Released as v1.6.0 but never merged to the default branch; folded in here (78dad01)
- First automated tests in the workspace: concurrent open, migration rollback, corruption refusal, newer-schema refusal, backup round-trip through an un-checkpointed WAL, fresh-open idempotence. Each was observed failing against the previous code (d781fe2)

### Documentation
- Spec-kit feature `002-db-durability-hardening` — specification, plan, research, data model, CLI contract, quickstart, task list — and the project constitution the plan is gated against (8d799ce)
- README: backup section with the reason file-level copying destroys a WAL database, and the manual restore procedure (d781fe2)

### Notes
- **v1.6.0 is contained in this release.** Its tag pointed at a commit that never reached the default branch, so the repository read as 1.5.0 while the installed binary reported 1.6.0. This release supersedes it.
- Verified against the preserved database from the 2026-07-27 incident: 22 consecutive operations, zero bytes changed, and both the refusal and the report name the file-level-copy signature in plain words.

---

## [v1.6.0] — 2026-06-21

### Added
- **Skills subsystem** — reusable procedural "how-to" cards with progressive disclosure. 4 MCP tools:
  - `skill_save` — create/update a skill (upsert by name). Trigger → FTS-indexed note (discoverable); body + tags → `data` (not keyword-indexed, so the body never pollutes search).
  - `skill_list` — cheap index (name + trigger + tags + uses), ranked by usage. Optional FTS `query` / `tag` filter.
  - `skill_get` — full markdown body by name, with fuzzy FTS fallback + `other_matches` for disambiguation. Bumps `access_count`.
  - `skill_remove` — delete a skill by name.
- New node type `Skill`.
- Skills surface automatically in `memory_recall` (new `skills` bucket) and `memory_status` (top skills by usage).
- **SessionStart hook** (`aurelius-skills.sh` / `au skills --hook`) injects the skill index into context every session via `hookSpecificOutput.additionalContext`.

### Changed
- CLI: new `au skills [--hook]` command.

---

## [v1.5.0] — 2026-04-19

### Added
- `memory_merge` — merge two duplicate/related nodes into one: rewires all edges from source to target, removes self-loops and duplicate edges, appends source's note to target, deletes source. CLI: `au merge <source> <target>` (909b06d)
- `task_stats` — analytics over tasks: counts by status/priority, completion rate, avg/median active→done duration, currently blocked count, oldest active age, done-in-window. CLI: `au task stats [--project] [--since-days]` (909b06d)

### Documentation
- Update tool count 19→21 in README and CLAUDE.md (238792c)

---

## [v1.4.1] — 2026-04-01

### Added
- Auto-index project on first use — no manual `au init`/`au reindex` needed (73afe86)

### Documentation
- Update README to v1.4.0 — task management, 19 tools, new sections (32da52e)

---

## [v1.4.0] — 2026-04-01

### Added
- **Task management system** — 5 MCP tools (`task_create`, `task_update`, `task_list`, `task_log`, `task_view`) + full CLI (`au task`) (6857f8f)
- New node types: `Task`, `WorkLog`; new relations: `SubtaskOf`, `Blocks`
- Tasks as hub nodes: collect work logs, decisions, problems, solutions via `contains` edges
- Acceptance criteria, priority-based sorting, auto-activation on first log
- `memory_status` shows active tasks; `memory_session` accepts `tasks` param and returns active hints
- `memory_recall` includes tasks in grouped results

### Other
- Verify post-commit hook links to project (31b55e5)
- Clean test commits (6ea8c12)

---

## [1.3.0] — 2026-03-28

### Fixed
- **`memory_session` auto-creates project nodes** — sessions now create their project hub node if it doesn't exist, and link all child nodes (decisions, problems, solutions) to it via `belongs_to` edges. Previously, sessions silently skipped project linking when the project node was missing, leaving the graph fragmented.
- **Project filter includes hub node** — sidebar project filter now includes the project node itself, keeping the graph connected when filtering by project.

### Improved
- **Obsidian-style graph physics** — reworked force simulation: no node pinning after drag (nodes release back into simulation), gentle center force, stronger link forces for cluster cohesion. Drag a node and its neighbors follow naturally.
- **Cleaner graph labels** — only project nodes show labels by default; other nodes reveal labels on hover/select with neighbor highlighting.
- **Softer link styling** — links are subtle by default, brighten on highlight (Obsidian-inspired).
- **Smaller node sizes** — reduced node radii for cleaner visualization at scale.
- **Project navigation in sidebar** — new "Projects" section extracts project names from `[project-name]` label prefix, allows one-click project scoping.

### Removed
- Position persistence (localStorage pinning) — graph recalculates layout each session, matching Obsidian behavior.

---

## [1.0.0] — 2026-03-21

### Added
- **`memory_gc`** — garbage collection: removes duplicate edges, orphaned edges, and duplicate nodes (by content_hash)
- **`memory_status` project filter** — optional `project` parameter to scope decisions, problems, sessions to a specific project
- **`memory_search` since filter** — optional `since` parameter for time-based queries (`today`, `yesterday`, `7d`, `24h`, ISO 8601)
- **Batch BFS** — context traversal uses batch queries (`WHERE id IN (...)`) instead of N+1 per-node queries
- **Relevance-ranked search** — FTS results boosted by `access_count` for frequently accessed nodes
- **V3 migration** — composite indexes: `edges(to_id, relation)`, unique `edges(from_id, to_id, relation)`, `nodes(content_hash)`, `nodes(node_type, created_at)`
- **V4 migration** — rebuilt FTS5 index without `data` column to eliminate JSON key noise in search results
- **Edge deduplication** — `INSERT OR IGNORE` prevents duplicate edges on same `(from_id, to_id, relation)` triple

### Refactored
- **`graph.rs`** (531 lines) → `graph/{crud, search, traverse}.rs` — modular graph operations
- **`handlers.rs`** (594 lines) → `handlers/{crud, session, status}.rs` — modular MCP handlers

### Fixed
- Project-scoped `memory_status` now uses `search_typed` for proper SQL-level type+FTS filtering
- Project-scoped `open_problems` uses `get_unsolved_problems` with label prefix filter
- FTS5 bracket escaping for `[project]` prefix queries
- V3 migration cleans duplicate edges before creating UNIQUE index

### Removed
- Dead code: unused `get_edges` single-node query (replaced by batch version)
- TimeForged sync — evaluated and rejected (time data not useful for AI memory)

---

## [0.5.0] — 2026-03-21

### Optimized
- **`memory_status`** — uses SQL LIMIT instead of fetching all nodes and truncating in Rust; 6x fewer rows deserialized
- **`get_unsolved_problems`** — parameterized node types (no hardcoded JSON strings), added LIMIT
- **`memory_session`** — deduplication via SHA-256 content_hash; duplicate calls return existing session instead of creating duplicates
- **`memory_session`** — removed double storage: decisions/problems no longer stored in Session node's `data` JSON (they're already separate graph nodes)

### Added
- `find_node_by_content_hash()` — lookup nodes by content hash for dedup

---

## [0.4.0] — 2026-03-21

### Added
- **`memory_recall`** — smart topic recall: combines FTS search with BFS traversal, returns results grouped by type (decisions, problems, solutions, sessions, other). One call instead of separate search+context
- **`memory_search` type filter** — optional `type` parameter to filter results by node type (e.g. `type: "decision"`)
- **`get_unsolved_problems()`** — SQL query that finds problems without a linked solution (via `solves` edge)
- **`search_typed()`** — FTS search with node type filter in core

### Improved
- **`memory_status`** — `open_problems` now shows only unsolved problems (those without a `solves` edge from a Solution node), not all problems
- **Web UI** — graph physics now always active (`cooldownTicks=Infinity`, `d3AlphaMin=0`), Obsidian-like behavior

### Fixed
- Graph visualization froze after 5-10 seconds due to d3-force simulation cooling down

---

## [0.3.0] — 2026-03-21

### Added
- **`memory_session`** — record session summaries with decisions, problems solved, and next steps; creates episodic Session node linked to project, plus Decision and Problem/Solution nodes with proper graph relations
- **`memory_update`** — update existing node's note and/or data by UUID or label; enables enriching nodes with additional context after creation
- **`memory_add` enhanced** — now accepts `data` (arbitrary JSON metadata) and `memory_kind` (semantic/episodic) parameters

### Improved
- **`memory_status`** — now returns recent solutions alongside problems, session details with full node info (not just brief), and uses lightweight count queries for stats
- **`memory_add`** — uses `add_node_full` internally, supporting all node fields

---

## [0.2.0] — 2026-03-21

### Improved
- **`memory_search`** — empty query (`""`) or wildcard (`"*"`) now returns most recent nodes instead of FTS5 error
- **`memory_dump`** — added pagination with `offset` and `limit` parameters (default: 50 items) to prevent exceeding MCP token limits; response includes `total_nodes`/`total_edges` counts for navigation

### Added
- `get_recent_nodes()` — fetch N most recent nodes by creation date
- `get_nodes_paginated()` / `get_edges_paginated()` — paginated graph queries
- `count_nodes()` / `count_edges()` — lightweight count queries

---

## [0.1.0] — 2026-03-21

### Added
- **Knowledge Graph Core** — SQLite-backed graph with FTS5 full-text search, WAL mode, versioned migrations
- **Domain Model** — 14 node types (Project, Crate, File, Decision, Concept, Problem, Solution, etc.), 16 relation types, MemoryKind (Semantic/Episodic)
- **Graph Operations** — add/delete/update nodes, BFS traversal, FTS search, touch (access tracking), find by label/data field
- **Project Indexer** — parses Cargo.toml workspaces, discovers crates, files, dependencies; SHA256 content hashing for incremental re-index
- **TimeForged Connector** — async integration with TimeForged time tracking daemon; pulls sessions, projects, languages into the graph
- **MCP Server** — JSON-RPC 2.0 over stdio, 8 tools: `memory_status`, `memory_context`, `memory_search`, `memory_add`, `memory_relate`, `memory_index`, `memory_forget`, `memory_dump`
- **CLI (`au`)** — 9 subcommands: `init`, `note`, `context`, `search`, `sync`, `reindex`, `view`, `export`, `mcp`, `touch`
- **Web UI** — React + TypeScript + Tailwind CSS + react-force-graph-2d; interactive graph visualization with Obsidian-style physics, sidebar filters, node detail panel, search
- **Claude Code Integration** — MCP server config, PostToolUse hook (tracks file access), Stop hook (auto re-index on session end), git post-commit hook (captures decisions)
- **Install script** — `install.sh` for one-command setup: build, install, configure hooks
