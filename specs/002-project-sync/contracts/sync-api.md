# Contract: Sync Server HTTP API

Exposed by `aurelius-sync-server`, mounted under `/sync` (deployed at
`https://aurelius.boostix.space/sync`). JSON over HTTPS.

## Authentication

Every push/pull request carries `Authorization: Bearer {uuid}:{secret}`,
where `uuid`/`secret` are a `CollaboratorGrant` credential pair (see
data-model.md). The server splits on the first `:`, looks up the grant by
`uuid`, hashes the supplied `secret` and compares to the stored
`secret_hash`, and checks `revoked_at IS NULL`. A valid credential
unambiguously identifies exactly one `(project_label, person)` — **the
project is never a separate client-supplied parameter**; it's always derived
from the credential. This is what lets the client-side flow be a single
`au sync {server} {uuid} {secret}` command with no project name to type.

**Errors** (both endpoints): `401` (unknown uuid, wrong secret, or revoked).

## `POST /sync/push`

Client sends everything new/changed locally since its last successful push.

**Request body**:

```json
{
  "nodes": [ { "...": "full Node, per data-model.md" } ],
  "edges": [ { "...": "full Edge, per data-model.md" } ]
}
```

- Every node/edge MUST already carry `created_by`; server rejects (`422`) any
  item missing attribution.
- Server upserts each node/edge by `id`, into the project the credential
  resolves to:
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

**Errors**: `401`, `422` (malformed payload or missing attribution).

## `GET /sync/pull?since={seq}`

- `since` omitted or `0` → full bootstrap: every live and tombstoned
  node/edge for the credential's project (satisfies FR-004), plus that
  project's label so a first-time client can create/attach its local project
  under the same name without having to already know it.
- `since={seq}` → only items with `sync_seq > seq`.

**Response** `200`:

```json
{
  "project": "aurelius",
  "nodes": [ { "...": "Node, including deleted_at for tombstones" } ],
  "edges": [ { "...": "Edge" } ],
  "server_seq": 1181
}
```

- Client persists `server_seq` as its new `SyncConfig.last_seq`, and
  `project` as `SyncConfig.project_label`, after successfully applying the
  response.
- An empty `nodes`/`edges` array with an unchanged `server_seq` means
  "nothing new" — not an error.

**Errors**: `401`.

## `POST /sync/grants` (admin-only, credential issuance)

Used by `au sync issue <project> --for "Name <email>"` (owner-side) to mint a
new collaborator credential. Protected by a separate admin credential (the
`AURELIUS_SYNC_ADMIN_TOKEN` the server was started with), not a
`CollaboratorGrant` — issuing access to a project is an administrative act,
not a synced-data operation.

**Request headers**: `Authorization: Bearer {admin_token}`

**Request body**:

```json
{ "project": "aurelius", "person_name": "Tester", "person_email": "tester@example.com" }
```

**Response** `200` (the secret is returned exactly once — the server only
ever stores its hash):

```json
{ "uuid": "b3f1...", "secret": "9c2a..." }
```

**Errors**: `401` (bad/missing admin token), `422` (malformed payload).

## Failure semantics (push/pull)

- Network failure, timeout, or `5xx`: caller (client) logs a warning and
  proceeds with normal local operation — sync is best-effort at session
  boundaries (FR-006, FR-011). No retry loop blocks the caller.
- A revoked credential (`CollaboratorGrant.revoked_at IS NOT NULL`) is
  rejected with `401` on every call, indistinguishable from an unknown
  credential.
