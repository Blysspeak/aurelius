# Implementation Plan: Two-Way Project Sync Between Aurelius Instances

**Branch**: `002-project-sync` | **Date**: 2026-08-01 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/002-project-sync/spec.md`

**Note**: This template is filled in by the `/speckit-plan` command; its definition describes the execution workflow.

## Summary

Let an Aurelius project owner opt a single project into two-way sync with one
or more personally-invited collaborators, through a self-hosted sync server
(new `aurelius-sync-server` crate, reusing `aurelius-core` for storage) that
each client's `au`/`aurelius` binaries push to and pull from at session
boundaries. Every synced node/edge carries git-style creator/modifier
attribution from a new local identity config, deletions propagate as
tombstones, and same-record conflicts resolve by last-writer-wins with the
losing edit retained for recovery. See `research.md` for the technology
decisions and `data-model.md` / `contracts/sync-api.md` for the schema and
API this plan builds toward.

## Technical Context

**Language/Version**: Rust, workspace edition 2021 (matches existing `Cargo.toml`)

**Primary Dependencies**: `axum` 0.8 (server HTTP API — already a workspace dependency via `au`'s web UI), `reqwest` 0.12 (client push/pull calls — already used by the Brave Search client and the `TimeForged` connector), `tokio`, `rusqlite` (bundled) — all pre-existing workspace dependencies; no new external crates required.

**Storage**: SQLite + WAL via `aurelius-core`, reused as-is for both client instances and the sync server's own database (a fourth SQLite file, server-local, holding only synced-project data — see research.md #2).

**Testing**: `cargo test`, matching existing workspace convention. Unit tests for upsert/LWW merge logic in `aurelius-core::sync::merge`; integration tests spin up `aurelius-sync-server` against a temp SQLite file and exercise push/pull over real HTTP via `reqwest`.

**Target Platform**: Server — Linux (Docker container on the owner's `boostix` VPS). Clients — same platforms `au`/`aurelius` already support (developer machines); no new platform constraints introduced.

**Project Type**: Multi-crate Rust workspace (extends the existing `aurelius-core` / `aurelius` / `au` structure with one new crate, `aurelius-sync-server`).

**Performance Goals**: Not throughput-critical. A handful of collaborators per project, pushes/pulls of at most a few hundred nodes/edges per session boundary. No specific req/s target — correctness and simplicity take priority over throughput (matches the "not for the masses" scope in spec.md's Assumptions).

**Constraints**: A temporarily unreachable sync server MUST NOT block local reads/writes on either client (FR-006, FR-011) — push/pull are always best-effort, fire-and-log-a-warning-on-failure operations, never in the critical path of a normal `au`/MCP command. The server MUST preserve `aurelius-core`'s existing single-writer-per-SQLite-file discipline (no new concurrent-write pattern introduced).

**Scale/Scope**: One owner, a small (single-digit) number of personally-invited collaborators, per shared project. Explicitly not designed for public/multi-tenant scale (no self-service signup, no dashboard — see spec.md Assumptions).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

`.specify/memory/constitution.md` has not been ratified for this project yet
(still template placeholders — no `/speckit-constitution` run to date). No
formal gates apply. In its place, this plan holds itself to the project's
existing observable conventions (README "Key Design" section and
`karpathy-guidelines`/`rust-clean-code` skills already governing this repo):
SQLite+WAL local-first storage, UUID primary keys, additive numbered
migrations (this feature adds migration V6), modular single-purpose crates,
no `unwrap`/`expect`/`panic!` on runtime paths, and surgical changes scoped
to what the feature needs. No violations to justify in Complexity Tracking.

*Re-checked after Phase 1 design (this plan + data-model.md + contracts):
still holds — no new external dependencies, no schema outside one additive
migration, no new crate beyond the one purpose-built sync server.*

## Project Structure

### Documentation (this feature)

```text
specs/002-project-sync/
├── plan.md              # This file (/speckit-plan command output)
├── research.md          # Phase 0 output (/speckit-plan command)
├── data-model.md        # Phase 1 output (/speckit-plan command)
├── quickstart.md        # Phase 1 output (/speckit-plan command)
├── contracts/
│   └── sync-api.md      # Phase 1 output (/speckit-plan command)
└── tasks.md             # Phase 2 output (/speckit-tasks command - NOT created by /speckit-plan)
```

### Source Code (repository root)

```text
crates/
├── aurelius-core/
│   └── src/
│       ├── models.rs         # + created_by/updated_by/deleted_at/sync_seq on Node & Edge
│       ├── db.rs              # + migration V6 (columns above, sync_config, collaborator_grants tables)
│       ├── identity.rs        # NEW: read/write ~/.config/aurelius/identity.toml
│       └── sync/
│           ├── mod.rs          # NEW: shared request/response types (SyncPushRequest, SyncPullResponse, ...)
│           └── merge.rs        # NEW: upsert-by-id + LWW conflict resolution (used only by the server)
├── aurelius-sync-server/       # NEW crate
│   ├── Cargo.toml
│   ├── Dockerfile
│   └── src/
│       ├── main.rs             # axum app bootstrap, config (port, db path)
│       ├── routes.rs           # POST /sync/push, GET /sync/pull
│       └── auth.rs             # Bearer token -> hash -> CollaboratorGrant lookup, + admin-token check for /sync/grants
├── aurelius/
│   └── src/
│       └── mcp/handlers/status.rs, session.rs   # memory_status triggers pull; memory_session triggers push, for shared projects only
└── au/
    └── src/
        └── commands.rs          # + `au identity set`, `au share issue|<server> <token>|disable|push|pull` subcommands
contrib/
└── claude-code/
    └── aurelius-reindex.sh      # extended: push after reindex, for projects with sync enabled
deploy/
└── aurelius-sync-server/
    ├── docker-compose.yml       # deployment unit for the boostix VPS
    └── README.md                # deploy/runbook notes (aurelius.boostix.space/sync reverse-proxy target)
```

**Structure Decision**: Single Rust workspace, Option 1 shape (no
frontend/backend split — `ui/` is unrelated to this feature and untouched).
One new crate (`aurelius-sync-server`) added alongside the existing three,
following the workspace's existing one-crate-per-concern convention. Sync
logic that must be shared between client and server (request/response types,
merge rules) lives in `aurelius-core::sync`, the crate both the clients and
the server already depend on — avoids a dependency cycle between
`aurelius-sync-server` and `au`/`aurelius`.

## Complexity Tracking

*No entries — Constitution Check reported no violations requiring justification.*
