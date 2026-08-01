# Phase 1 Data Model: Two-Way Project Sync

`aurelius-core`'s schema is shared by every binary (`au`, `aurelius`, and the
new `aurelius-sync-server`). Migration **V6** adds the columns/tables below.
Client instances and the sync server populate different subsets of the same
schema rather than using divergent schemas — see research.md #2.

## Extended: `Node` (existing entity, `crates/aurelius-core/src/models.rs`)

| Field | Type | Notes |
|---|---|---|
| *(existing fields unchanged)* | | id, node_type, label, note, source, data, created_at, updated_at, memory_kind, last_accessed_at, access_count, content_hash |
| `created_by` | `Option<String>` | New. Git-style `"Name <email>"`, stamped from the local identity config at creation. `None` for nodes created before this migration. |
| `updated_by` | `Option<String>` | New. Same format; overwritten on every update. |
| `deleted_at` | `Option<DateTime<Utc>>` | New. Soft-delete tombstone. `memory_forget` sets this instead of issuing `DELETE`. `NULL` = live. |
| `sync_seq` | `Option<i64>` | New. Set only by the sync server on upsert (monotonic, shared across all synced projects on that server). Always `NULL` on a client's own local rows until they've round-tripped through a sync. |

**Conflict bookkeeping**: when the server detects two updates to the same
`id` racing each other, the losing update's `note`/`data`/`updated_by`/
`updated_at` are preserved under a reserved key in the *winning* row's
`data` JSON: `data._sync_conflict = { note, data, updated_by, updated_at }`.
This is overwritten if another conflict happens later on the same node — only
the most recent losing edit is retained. This is a display/recovery aid, not
a new relation to traverse.

## Extended: `Edge` (existing entity)

| Field | Type | Notes |
|---|---|---|
| *(existing fields unchanged)* | | id, from_id, to_id, relation, weight, created_at |
| `created_by` | `Option<String>` | New, same format as `Node.created_by`. |
| `deleted_at` | `Option<DateTime<Utc>>` | New. Set in lockstep when either endpoint `Node` is soft-deleted (cascade), so edges tombstone together with their node rather than being computed at sync time. |
| `sync_seq` | `Option<i64>` | New, same semantics as `Node.sync_seq`. |

## New: `SyncConfig` (client-side only, one row per project)

Not a graph node — operational config, local to a client instance.

| Field | Type | Notes |
|---|---|---|
| `project_label` | `TEXT` (PK) | Matches the existing project label convention. Learned from the server's response during connect (see below), not typed in advance by a collaborator. |
| `server_url` | `TEXT` | Normalized to `https://{host}/sync`; e.g. given host `aurelius.boostix.space`, stored as `https://aurelius.boostix.space/sync`. |
| `token` | `TEXT` | This client's single credential for that project — the token itself resolves to exactly one `(project_label, person)` server-side (see `CollaboratorGrant`), so no separate project id/name is ever transmitted alongside it. |
| `enabled` | `BOOLEAN` | Sync on/off for this project; opt-in, defaults off. |
| `last_seq` | `INTEGER` | Highest `sync_seq` this client has pulled for this project. Drives incremental `GET /sync/pull`. |
| `updated_at` | `TEXT` | Bookkeeping. |

A project with no `SyncConfig` row, or `enabled = false`, is never touched by
sync — satisfies FR-001/FR-002.

## New: `CollaboratorGrant` (server-side only, one row per issued token)

| Field | Type | Notes |
|---|---|---|
| `token_hash` | `TEXT` (PK) | SHA-256 of the plaintext token (workspace already depends on `sha2`, used elsewhere for `content_hash`). Looked up directly by hash — an incoming request's token is hashed and matched against this column, so the plaintext is never stored server-side; it's shown once at issuance and lives only on the holder's own machine. |
| `person_name` | `TEXT` | For audit/logging on the server side; the actual attribution on synced data comes from each node/edge's own `created_by`/`updated_by`, not from this table. |
| `person_email` | `TEXT` | |
| `project_label` | `TEXT` | Which single shared project this credential may push/pull. A collaborator with access to two projects holds two credentials. |
| `granted_at` | `TEXT` | |
| `revoked_at` | `Option<TEXT>` | Revocation stops future push/pull with this credential; does not retract data already delivered (matches the spec's Edge Cases section). |

## New: local identity config (file, not a DB table)

`~/.config/aurelius/identity.toml`, one per machine, independent of any
project — mirrors `dirs_next::config_dir()` usage already established by
`~/.config/aurelius/brave.key`.

```toml
name = "Vladislav Rahmanov"
email = "blysspeak@gmail.com"
```

Read once per process start; every node/edge a client creates or updates is
stamped `"{name} <{email}>"` into `created_by`/`updated_by`. Required before
`au share` can be used to connect a project (fails fast with a clear error if
unset), but is independent of sync itself — it's the durable identity
referenced by FR-007.

## State transitions

- **Node/Edge**: `live` → `deleted` (via `deleted_at`), one-way. A
  soft-deleted row is never revived by sync; a client that still has an old
  live copy will receive the tombstone on next pull and soft-delete its own
  copy to match.
- **SyncConfig.enabled**: `off` → `on` (via `au share <server> <uuid> <secret>`)
  triggers a full bootstrap pull (FR-004); `on` → `off` (`au share disable
  <project>`) stops future sync but does not delete already-synced local data.
- **CollaboratorGrant**: `active` (`revoked_at IS NULL`) → `revoked`. Checked
  on every push/pull request; a revoked credential is rejected.
