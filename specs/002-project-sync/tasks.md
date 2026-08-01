# Tasks: Two-Way Project Sync Between Aurelius Instances

**Input**: Design documents from `/specs/002-project-sync/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/sync-api.md, quickstart.md (all present)

**Tests**: Included — plan.md's Technical Context specifies `cargo test` coverage for the merge/conflict logic and a push/pull round-trip, and this is the workspace's project-wide "done" bar (fmt + clippy + tests green).

**Organization**: Tasks are grouped by user story (spec.md P1-P4) to enable independent implementation and testing of each.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependency on an incomplete task)
- **[Story]**: Which user story this task belongs to (US1-US4)

---

## Phase 1: Setup

**Purpose**: Stand up the new crate before any feature logic lands in it.

- [ ] T001 Add `crates/aurelius-sync-server` to the `members` list in `Cargo.toml`, create `crates/aurelius-sync-server/Cargo.toml` depending on `aurelius-core` (path), `axum`, `tokio`, `serde`, `serde_json`, `anyhow`, `tracing`, `tracing-subscriber`, `clap` (all already workspace deps — see research.md #3)
- [ ] T002 [P] Scaffold `crates/aurelius-sync-server/src/main.rs`: clap args (`--port`, `--db <path>`), tracing init, empty axum `Router` that binds and serves
- [ ] T003 [P] Scaffold `deploy/aurelius-sync-server/Dockerfile` (multi-stage: `cargo build --release -p aurelius-sync-server`, slim runtime image) and `deploy/aurelius-sync-server/docker-compose.yml` (port + volume for the server's SQLite file)

**Checkpoint**: `cargo build -p aurelius-sync-server` succeeds and produces a binary that starts and listens.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Schema, identity, and shared types every user story needs. No user story can be implemented before this phase is done.

- [ ] T004 Add migration V6 in `crates/aurelius-core/src/db.rs`: `nodes` gains `created_by TEXT`, `updated_by TEXT`, `deleted_at TEXT`, `sync_seq INTEGER`; `edges` gains `created_by TEXT`, `deleted_at TEXT`, `sync_seq INTEGER`; new tables `sync_config` and `collaborator_grants` per data-model.md
- [ ] T005 Extend `Node`/`Edge` structs in `crates/aurelius-core/src/models.rs` with the new optional fields, and update the row-mapping code in `crates/aurelius-core/src/graph/crud.rs` to read/write them (depends on T004)
- [ ] T006 [P] Create `crates/aurelius-core/src/identity.rs`: `Identity { name, email }`, load/save `~/.config/aurelius/identity.toml` via `dirs_next::config_dir()` (matches the existing `brave.key` pattern in `crates/aurelius/src/search/brave.rs`), `Identity::as_author() -> String` returning `"Name <email>"`
- [ ] T007 [P] Create `crates/aurelius-core/src/sync/mod.rs`: `SyncPushRequest`, `SyncPushResponse`, `SyncPullResponse` structs matching `contracts/sync-api.md`, `Serialize`/`Deserialize`
- [ ] T008 Create `crates/aurelius-core/src/sync/merge.rs`: `apply_push(conn, project, nodes, edges) -> SyncPushResponse` doing upsert-by-`id` with last-writer-wins by `updated_at`, retaining the losing version under `data._sync_conflict` per data-model.md; `pull_since(conn, project, since_seq) -> SyncPullResponse` (depends on T004, T005, T007) — include unit tests for: new-id insert, newer-wins overwrite, older-loses no-op, conflict retention field populated correctly
- [ ] T009 Stamp `created_by`/`updated_by` from `identity::current()` in the node/edge create and update paths of `crates/aurelius-core/src/graph/crud.rs` (depends on T005, T006)
- [ ] T010 Change the delete path in `crates/aurelius-core/src/graph/crud.rs` (backing `memory_forget`) from `DELETE` to setting `deleted_at`, cascading the same timestamp onto that node's edges instead of deleting them (depends on T005)

**Checkpoint**: `cargo build --workspace` succeeds; `cargo test -p aurelius-core` passes (merge unit tests from T008 green). No user-facing behavior has changed yet — this phase is pure plumbing.

---

## Phase 3: User Story 1 - Share a project and bootstrap a collaborator (Priority: P1) 🎯 MVP

**Goal**: Owner issues a collaborator a single token; the collaborator (or the owner, for their own project) runs one command, `au share <server> <token>`, and that instance receives the full existing history.

**Independent Test**: Run quickstart.md steps 1-4.

### Implementation for User Story 1

- [ ] T011 [P] [US1] Implement `POST /sync/push`, `GET /sync/pull`, `POST /sync/grants`, and `POST /sync/grants/revoke` handlers in `crates/aurelius-sync-server/src/routes.rs`, delegating push/pull to `sync::merge` (depends on T008, T007). `push`/`pull` derive their project from the authenticated token (no client-supplied `project` field); `grants` issues a new `collaborator_grants` row (random token, stored only as its sha256 `token_hash`) for a project; `grants/revoke` sets `revoked_at` on matching rows by `(project, person_email)`. Both `grants*` endpoints are admin-protected, separately from `push`/`pull` (see T012).
- [ ] T012 [P] [US1] Implement auth in `crates/aurelius-sync-server/src/auth.rs`: for `push`/`pull`, parse `Authorization: Bearer {token}`, hash it and look up `collaborator_grants` by `token_hash`, check `revoked_at IS NULL`, resolve `project_label` from the grant; for `grants`/`grants/revoke`, check `Authorization: Bearer {admin_token}` against the `AURELIUS_SYNC_ADMIN_TOKEN` env var the server was started with. Return `401`/`422` per contracts/sync-api.md.
- [ ] T013 [US1] Wire `routes.rs` + `auth.rs` into the `Router` in `crates/aurelius-sync-server/src/main.rs` (depends on T011, T012)
- [ ] T014 [P] [US1] Add `au identity set --name <name> --email <email>` subcommand: `crates/au/src/main.rs` (clap variant) + implementation in `crates/au/src/commands.rs` (depends on T006)
- [ ] T015 [US1] Add the ADMIN-side subcommands (require `AURELIUS_SYNC_ADMIN_TOKEN` in the environment) in `crates/au/src/main.rs` + `crates/au/src/commands.rs`:
  - `au share issue <project> --for "Name <email>" --server <host-or-url>`: **`<project>` MUST already exist locally — look it up with the existing `find_project_by_label` (do NOT find-or-create, unlike `au note -p`; minting access to the wrong/a brand-new empty project by typo is a real mistake, not a harmless label).** If no project matches, error out AND list existing project labels (via the existing `get_nodes_by_type(conn, &NodeType::Project)`) so the owner can immediately see the correct name to retype, e.g. `error: no project named "demoo" — did you mean one of: demo, aurelius, boostix?`. On a match, call `POST /sync/grants`, print the returned `token` for the owner to hand off out of band.
  - `au share revoke <project> --for <email> --server <host-or-url>`: calls `POST /sync/grants/revoke`, prints how many grants were revoked.
  (depends on T004, T007, T013)
- [ ] T016 [US1] Add the PARTICIPANT-side subcommands (everyone, owner included — no admin token needed) in `crates/au/src/commands.rs`:
  - `au share <server> <token>` (positional, no flags — every participant runs this once per project to connect it): normalizes `server` (bare host → `https://{host}/sync`; already-a-URL passed through as-is, so `http://localhost:PORT/sync` works for local testing), performs an initial `GET /sync/pull?since=0` using that token, reads the `project` field from the response to learn the project's name — this is intentionally the ONLY place the project name comes from on the connecting side, no separate project-selection step or typo risk here — creates/updates the local project and its `sync_config` row (`server_url`, `token`, `enabled=true`, `last_seq` from the response), and applies the bootstrapped nodes/edges. If a local project with that exact name already exists and isn't already this same `sync_config` row, still proceed (attach sync to it) but print a one-line notice rather than silently merging without any signal.
  - `au share push [project]` / `au share pull [project]`: HTTP round-trip via `reqwest` against the project's `sync_config` row. With no `[project]` given, act on every project with `enabled=true`; with one given, it must be sync-enabled or error.
  - `au share list`: prints every local `sync_config` row (project, server_url, enabled, last_seq, updated_at) — the at-a-glance "what's currently shared/connected" view.
  - `au share disable <project>`: sets that project's `sync_config.enabled = false` (local-only flip — future push/pull skip it; does not call the server, does not delete already-synced local data, matches data-model.md's state transitions).
  (depends on T007, T013, T015)
- [ ] T017 [P] [US1] Integration test in `crates/aurelius-sync-server/tests/push_pull.rs`: start the server against a temp SQLite file, push a node/edge from a fake "owner" payload, pull with `since=0` from a fake "collaborator" and assert full history round-trips (depends on T011-T016)

**Checkpoint**: US1 works end-to-end per quickstart.md steps 1-4.

---

## Phase 4: User Story 2 - See each other's ongoing work automatically (Priority: P2)

**Goal**: New decisions/tasks on either side appear on the other side by that person's next session, attributed.

**Independent Test**: Run quickstart.md step 5 and step 10.

### Implementation for User Story 2

- [ ] T018 [US2] In `crates/aurelius/src/mcp/handlers/status.rs`, before building the `memory_status` response for a project with `sync_config.enabled`, call the sync pull path from T016 (depends on T016)
- [ ] T019 [US2] In `crates/aurelius/src/mcp/handlers/session.rs`, after a `memory_session` write completes for a project with `sync_config.enabled`, call the sync push path from T016 (depends on T016)
- [ ] T020 [P] [US2] Extend `contrib/claude-code/aurelius-reindex.sh` (Stop hook) to also invoke `au share push` for any project with sync enabled, as the CLI-level equivalent of T019 for non-MCP flows (depends on T016)
- [ ] T021 [P] [US2] Surface `created_by`/`updated_by` in the output built by `crates/aurelius/src/mcp/handlers/search.rs`, `status.rs`, and `task.rs` (depends on T005)
- [ ] T022 [US2] Make push/pull failures non-blocking: catch and `tracing::warn!` rather than propagate, at every call site from T018-T020 (depends on T018, T019, T020)

**Checkpoint**: US2 works end-to-end per quickstart.md step 5; killing the server and repeating quickstart.md step 10 confirms local work is unaffected.

---

## Phase 5: User Story 3 - Bug report becomes a tracked, attributed task (Priority: P3)

**Goal**: A task filed on one side, worked and completed on the other, round-trips with full attribution.

**Independent Test**: Run quickstart.md step 6.

### Implementation for User Story 3

- [ ] T023 [US3] Verify `task_create`/`task_update`/`task_log` handlers in `crates/aurelius/src/mcp/handlers/task.rs` go through the crud path stamped in T009 (no special-casing needed — tasks are nodes); add creator/last-actor display to `au task show` / `au task list` in `crates/au/src/commands.rs` (depends on T009, T016)
- [ ] T024 [P] [US3] Integration test in `crates/aurelius-sync-server/tests/task_roundtrip.rs`: simulate a task pushed by a "tester" client, pulled and completed by an "owner" client, pulled back by the tester, asserting status and attribution at each hop (depends on T016, T023)

**Checkpoint**: US3 works end-to-end per quickstart.md step 6.

---

## Phase 6: User Story 4 - Deletions and overlapping edits don't corrupt shared history (Priority: P4)

**Goal**: Tombstones propagate and don't resurrect; concurrent edits resolve deterministically without silent data loss.

**Independent Test**: Run quickstart.md steps 7-8.

### Implementation for User Story 4

- [ ] T025 [US4] Confirm tombstones (`deleted_at`) round-trip correctly through `routes.rs`/`merge.rs` end-to-end, and that `au forget` (or equivalent MCP `memory_forget`) results in the deletion going out on the next push — `crates/au/src/commands.rs` (depends on T010, T016)
- [ ] T026 [P] [US4] Additional unit tests in `crates/aurelius-core/src/sync/merge.rs` for the races called out in spec.md Edge Cases: simultaneous update-vs-update, and update-vs-tombstone ordering (depends on T008)
- [ ] T027 [P] [US4] Surface `data._sync_conflict` (when present) in `au context <project> -v` output — `crates/au/src/commands.rs` (depends on T008)

**Checkpoint**: US4 works end-to-end per quickstart.md steps 7-8.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Deployment readiness, documentation, and workspace-wide quality gates.

- [ ] T028 [P] Write `deploy/aurelius-sync-server/README.md` as a GENERIC template guide — anyone should be able to copy `deploy/aurelius-sync-server/` and self-host their own private instance with no code changes: prerequisites, `cp .env.example .env` + fill in `AURELIUS_SYNC_ADMIN_TOKEN`, `docker compose up -d --build`, how to run `au share issue`/`revoke` against it, and a reverse-proxy note (works behind any TLS-terminating proxy, not boostix-specific). End the README with a short, clearly-separated "This project's own deployment" subsection noting *this* repo's instance runs on the owner's `boostix` VPS at `aurelius.boostix.space/sync` — **that subsection is documentation only; do not execute against the real boostix host as part of this task list** (see note below). `.env.example` and the Dockerfile/docker-compose.yml already exist from Phase 1 — sanity-check them against what you actually built (e.g. do routes.rs/auth.rs need any env var this compose file doesn't already pass through?) and fix if not, but don't rewrite what already works.
- [ ] T029 [P] Update `README.md`: document the sync feature, `au identity` / `au share` commands, and migration V6, following the existing "Key Design" / "CLI" section conventions
- [ ] T030 Run `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` across the workspace; fix all findings
- [ ] T031 Run `cargo test --workspace`; fix any failures
- [ ] T032 Execute `quickstart.md` end-to-end (all 10 steps) against a locally-run `aurelius-sync-server`; fix any gaps found

**Note on T028**: actually deploying to the real `boostix` VPS (SSH access, DNS/reverse-proxy for `aurelius.boostix.space`, issuing the real production collaborator token) is a shared-infrastructure change and is intentionally left as an owner-confirmed action outside this task list, not something to run unattended.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies.
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS every user story.
- **User Story 1 (Phase 3)**: Depends on Foundational. Delivers the actual push/pull mechanism, so Stories 2-4 build on its code (T011-T016), not just its phase ordering — that dependency is intentional per spec.md ("User Story 2... depends on User Story 1 (bootstrap) already working").
- **User Story 2 (Phase 4)**: Depends on US1 (T016). Independently testable once US1 is done.
- **User Story 3 (Phase 5)**: Depends on US1 (T016) and T009. Independently testable once US1 is done; does not require US2.
- **User Story 4 (Phase 6)**: Depends on US1 (T016) and T010. Independently testable once US1 is done; does not require US2 or US3.
- **Polish (Phase 7)**: Depends on all four user stories being complete.

### Parallel Opportunities

- T002, T003 in parallel once T001 lands.
- T006, T007 in parallel once T004/T005 land (T008 needs both).
- T011, T012 in parallel; both needed before T013.
- T014 in parallel with T011-T013.
- T020, T021 in parallel with T018/T019.
- T024, T026, T027 in parallel with each other.
- T028, T029 in parallel during Polish.

---

## Parallel Example: Foundational Phase

```
Task: "Create crates/aurelius-core/src/identity.rs"
Task: "Create crates/aurelius-core/src/sync/mod.rs"
```

## Parallel Example: User Story 1

```
Task: "Implement POST /sync/push and GET /sync/pull in crates/aurelius-sync-server/src/routes.rs"
Task: "Implement bearer-token auth in crates/aurelius-sync-server/src/auth.rs"
Task: "Add `au identity set` subcommand"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Phase 1 (Setup) → Phase 2 (Foundational) → Phase 3 (US1).
2. **STOP and VALIDATE**: run quickstart.md steps 1-4 against two local instances and a locally-run server.
3. This alone proves the mechanism end-to-end (bootstrap + basic push/pull) before layering on the automatic-hook and edge-case work.

### Incremental Delivery

1. Setup + Foundational → foundation ready, nothing user-visible yet.
2. US1 → collaborators can be bootstrapped and manually `au share push/pull` (MVP).
3. US2 → sync becomes automatic at session boundaries; no more manual `au share push/pull` needed day-to-day.
4. US3 → validates the actual motivating workflow (bug report → tracked task) end-to-end.
5. US4 → hardens deletion and conflict handling.
6. Polish → deployable image, docs, workspace-wide `fmt`/`clippy`/`test` gate.

---

## Notes

- No cross-story same-file conflicts by design: Phase 2 owns `models.rs`/`db.rs`/`crud.rs`/`identity.rs`/`sync/`; Phase 3 owns the new `aurelius-sync-server` crate plus `au`'s new subcommands; Phases 4-6 touch existing MCP handlers and `au` commands but in non-overlapping functions.
- Commit after each phase checkpoint, not after every single task.
- `T028`'s actual production deploy step is owner-confirmed, not autonomous — flag it back to the owner rather than executing it.
