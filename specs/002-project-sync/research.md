# Phase 0 Research: Two-Way Project Sync

All technical unknowns below were resolved through direct discussion with the
project owner before this plan was written (see conversation leading to
`spec.md`). No open `NEEDS CLARIFICATION` markers remain in the Technical
Context. This document records the decisions in research format for
traceability.

## 1. Server topology

**Decision**: Hub-and-spoke through a single self-hosted sync server, not
direct peer-to-peer between client machines.

**Rationale**: Only two participants exist today (owner + one tester), but the
owner wants to add more collaborators later without redesigning anything.
Hub-and-spoke means every new collaborator repeats the same bootstrap flow
against the same server; a mesh topology would need to renegotiate
peer-to-peer connections for every pair of participants. The owner already
operates a VPS (`boostix`) reachable by SSH, removing the main cost of running
a small always-on service.

**Alternatives considered**:
- Direct P2P (e.g. over a Tailscale/WireGuard tunnel) — rejected: requires
  both machines to be online simultaneously for a push to land, and doesn't
  scale to N collaborators without a mesh of tunnels.
- A shared git repository as the transport, with sync state exported as
  NDJSON and merged via git — rejected: was the initial working assumption,
  but the owner explicitly changed direction mid-design toward a purpose-built
  server, wanting real push/pull semantics rather than git-merge semantics.

## 2. Server implementation approach

**Decision**: The sync server reuses `aurelius-core` (the existing graph
storage crate) as its storage engine, wrapped in a thin HTTP API (new
`aurelius-sync-server` crate). It holds a full copy of the graph for shared
projects only, and owns merge/conflict resolution centrally.

**Rationale**: `aurelius-core` already provides UUID-keyed nodes/edges, FTS,
and migrations — reimplementing storage for the server would duplicate that
work. Centralizing merge logic in one place (the server) avoids having to
keep two independent client implementations of the same upsert/LWW logic in
sync with each other.

**Alternatives considered**:
- Server as a dumb relay (append-only op log, no merge logic, clients merge
  locally) — considered and explicitly rejected in favor of the above: more
  code to duplicate and keep consistent across every client, for a server
  that is barely simpler to run.

## 3. HTTP framework

**Decision**: `axum`.

**Rationale**: Already a workspace dependency (`au`'s local graph-viewer web
UI on port 7175 uses it), so no new dependency is introduced. Consistent with
existing code style and the `tokio` async runtime already used throughout the
workspace.

**Alternatives considered**: `actix-web`, `warp` — both viable but would add a
second web framework to the workspace for no functional benefit.

## 4. Transport client

**Decision**: `reqwest` (already a workspace dependency, used by the Brave
Search client and the `TimeForged` connector) for the client-side push/pull
calls.

**Rationale**: Zero new dependencies; consistent with the existing
`Connector`-style pull pattern in `aurelius-core::connector`, even though this
feature needs push as well as pull and therefore does not implement that
trait directly.

## 5. Conflict resolution

**Decision**: Last-writer-wins per node, decided by `updated_at`, with the
losing version preserved (not discarded) in the winning node's `data` JSON
under a `sync_conflict_of` key.

**Rationale**: Matches FR-012 / User Story 4 in the spec. Usage is
additive-dominant (new decisions, new tasks, new work log entries), so
true same-node concurrent edits are expected to be rare; a full CRDT or
operational-transform merge would add significant complexity for a case that
happens occasionally rather than routinely, and the owner explicitly ruled
out automatic field-level merging as in-scope for v1.

**Alternatives considered**:
- CRDT-based field merge — rejected as unjustified complexity for v1 (see
  Assumptions in spec.md).
- Reject/block conflicting writes until manually resolved — rejected: would
  violate FR-011 (a temporarily unreachable or lagging peer must never block
  the other side's local work).

## 6. Change tracking / cursor

**Decision**: A monotonic `sync_seq` integer assigned by the server on every
upsert (per synced project), not a wall-clock timestamp. Each client stores
the last `sync_seq` it has pulled per (project, server).

**Rationale**: Clock skew between the owner's and collaborator's machines
cannot be trusted as an ordering signal for "what's new since last sync";
a server-assigned monotonic counter can. `updated_at` (wall-clock, client
-supplied) is still used only for the narrower LWW conflict tiebreak in
research item 5, not for cursoring.

## 7. Deletion propagation

**Decision**: Soft-delete (`deleted_at` timestamp) instead of hard `DELETE`,
on both client and server. A tombstone is a normal syncable row like any
other change, propagated and cursored the same way as an upsert.

**Rationale**: A hard delete leaves no record to propagate — the peer that
already has the old copy would never learn it should be removed, and worse,
could resurrect it by pushing its still-existing local copy back to the
server later. This directly satisfies FR-010.

## 8. Authentication

**Decision**: A single per-collaborator token, issued manually by the
project owner (no self-service signup, no OAuth). The server stores only
`sha256(token)`, never the plaintext, and looks up a request's token by its
hash directly. The token is handed to the collaborator out of band, who runs
one connect command, `au share {server} {token}`, to attach a project — no
separate project name to type, since the token alone resolves to exactly one
`(project, person)`.

**Rationale**: Matches FR-014 and the "not for the masses" constraint —
this serves the owner and a small number of personally-invited collaborators.
A single opaque token (rather than a public-id/secret pair) is the simplest
credential shape that still lets the server avoid persisting anything
usable on its own — hashing a single random token is exactly as safe as
hashing a secret half of a pair, with one fewer value for a collaborator to
copy around. Considered and rejected a split id+secret pair (more standard
for high-volume public APIs, e.g. AWS-style access-key/secret-key) as
unjustified complexity here. Token issuance and revocation are
owner-administered, out of band.

## 9. Deployment

**Decision**: `aurelius-sync-server` ships as a Docker image, deployed to the
owner's existing `boostix` VPS, reachable at `aurelius.boostix.space/sync`.

**Rationale**: Owner-specified target; the VPS is already reachable via the
owner's existing SSH tooling, and Docker keeps the service isolated from
whatever else runs on that host.
