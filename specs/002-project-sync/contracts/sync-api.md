# Contract: Sync Server HTTP API

Exposed by `aurelius-sync-server`, mounted under `/sync` (deployed at
`https://aurelius.boostix.space/sync`). JSON over HTTPS. All endpoints
require `Authorization: Bearer <token>`; the token maps to exactly one
`(project_label, person)` via `CollaboratorGrant` (see data-model.md).

## `POST /sync/push`

Client sends everything new/changed locally since its last successful push
for this project.

**Request body**:

```json
{
  "project": "aurelius",
  "nodes": [ { "...": "full Node, per data-model.md" } ],
  "edges": [ { "...": "full Edge, per data-model.md" } ]
}
```

- `project` MUST match the token's granted `project_label`; mismatch → `403`.
- Every node/edge MUST already carry `created_by`; server rejects (`422`) any
  item missing attribution.
- Server upserts each node/edge by `id`:
  - New `id` → insert, assign `sync_seq`.
  - Existing `id` with a newer `updated_at` than the server's stored copy →
    overwrite, preserve the losing version per the conflict rule in
    data-model.md, assign a new `sync_seq`.
  - Existing `id` with an older-or-equal `updated_at` → no-op (server's copy
    already wins; nothing pushed back into the response).
- A `deleted_at` present on a pushed item is treated as a tombstone write,
  not a real `DELETE`, and propagates like any other change.

**Response** `200`:

```json
{ "accepted": 42, "conflicts": 1, "server_seq": 1181 }
```

- `conflicts` counts items where the server's existing copy won over what the
  client pushed (see data-model.md conflict bookkeeping) — informational,
  not an error.

**Errors**: `401` (bad/unknown token), `403` (token/project mismatch), `422`
(malformed payload or missing attribution).

## `GET /sync/pull?project={label}&since={seq}`

- `since` omitted or `0` → full bootstrap: every live and tombstoned
  node/edge for the project (satisfies FR-004).
- `since={seq}` → only items with `sync_seq > seq`.
- `project` MUST match the token's granted `project_label`; mismatch → `403`.

**Response** `200`:

```json
{
  "nodes": [ { "...": "Node, including deleted_at for tombstones" } ],
  "edges": [ { "...": "Edge" } ],
  "server_seq": 1181
}
```

- Client persists `server_seq` as its new `SyncConfig.last_seq` after
  successfully applying the response.
- An empty `nodes`/`edges` array with an unchanged `server_seq` means
  "nothing new" — not an error.

**Errors**: `401`, `403`.

## Failure semantics (applies to both endpoints)

- Network failure, timeout, or `5xx`: caller (client) logs a warning and
  proceeds with normal local operation — sync is best-effort at session
  boundaries (FR-006, FR-011). No retry loop blocks the caller.
- A revoked token (`CollaboratorGrant.revoked_at IS NOT NULL`) is rejected
  with `401` on every call, indistinguishable from an unknown token.
