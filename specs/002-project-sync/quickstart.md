# Quickstart: Validate Two-Way Project Sync

Proves User Stories 1-4 end-to-end using two local client databases and a
locally-run copy of the sync server (no need to touch the real boostix
deployment to validate the feature).

## Prerequisites

- Workspace builds: `cargo build --workspace`
- Two separate local Aurelius data directories to stand in for "owner
  machine" and "collaborator machine":
  ```bash
  export AURELIUS_HOME_A=/tmp/aurelius-owner
  export AURELIUS_HOME_B=/tmp/aurelius-tester
  ```

## 1. Set identities (FR-007)

```bash
AURELIUS_HOME=$AURELIUS_HOME_A au identity set --name "Owner" --email "owner@example.com"
AURELIUS_HOME=$AURELIUS_HOME_B au identity set --name "Tester" --email "tester@example.com"
```

## 2. Start a local sync server

```bash
cargo run -p aurelius-sync-server -- --port 8181 --db /tmp/aurelius-sync-server.db
```

## 3. Seed the owner's project and issue a collaborator credential (User Story 1)

```bash
AURELIUS_HOME=$AURELIUS_HOME_A au note "chose axum for the sync API" -p demo
AURELIUS_HOME=$AURELIUS_HOME_A au task new "Ship v1" -p demo --priority high

# owner-side admin action against the server:
AURELIUS_SYNC_ADMIN_TOKEN=<set-when-starting-the-server> \
  AURELIUS_HOME=$AURELIUS_HOME_A au sync issue demo --for "Tester <tester@example.com>" --server localhost:8181
# prints a uuid + secret; hand both to the collaborator out of band

# the owner also connects their own project, same command every participant uses:
AURELIUS_HOME=$AURELIUS_HOME_A au sync localhost:8181 <owner-uuid-from-a-self-issued-grant> <owner-secret>
```

## 4. Bootstrap the collaborator (User Story 1)

```bash
AURELIUS_HOME=$AURELIUS_HOME_B au sync localhost:8181 <uuid-from-step-3> <secret-from-step-3>
AURELIUS_HOME=$AURELIUS_HOME_B au context demo
```

**Expected**: the tester's `demo` project shows the decision and the task
from step 3, with `created_by: Owner <owner@example.com>`.

## 5. Two-way propagation with attribution (User Story 2)

```bash
AURELIUS_HOME=$AURELIUS_HOME_B au note "found a login timeout bug" -p demo
AURELIUS_HOME=$AURELIUS_HOME_A au sync pull demo
AURELIUS_HOME=$AURELIUS_HOME_A au context demo
```

**Expected**: the owner sees the tester's note, attributed to
`Tester <tester@example.com>`.

## 6. Bug report → tracked task (User Story 3)

```bash
AURELIUS_HOME=$AURELIUS_HOME_B au task new "Fix login timeout" -p demo --priority high
AURELIUS_HOME=$AURELIUS_HOME_B au sync push demo
AURELIUS_HOME=$AURELIUS_HOME_A au sync pull demo
AURELIUS_HOME=$AURELIUS_HOME_A au task list --project demo   # shows task, filed by Tester
AURELIUS_HOME=$AURELIUS_HOME_A au task log <id> "fixed session TTL"
AURELIUS_HOME=$AURELIUS_HOME_A au task done <id>
AURELIUS_HOME=$AURELIUS_HOME_A au sync push demo
AURELIUS_HOME=$AURELIUS_HOME_B au sync pull demo
AURELIUS_HOME=$AURELIUS_HOME_B au task show <id>   # done, completed by Owner
```

## 7. Deletion propagates (User Story 4)

```bash
AURELIUS_HOME=$AURELIUS_HOME_A au forget <some-node-id>
AURELIUS_HOME=$AURELIUS_HOME_A au sync push demo
AURELIUS_HOME=$AURELIUS_HOME_B au sync pull demo
AURELIUS_HOME=$AURELIUS_HOME_B au context demo   # node is gone, does not reappear on a later pull
```

## 8. Conflict retains the losing edit (User Story 4)

```bash
# Both sides edit the same node before either syncs
AURELIUS_HOME=$AURELIUS_HOME_A au sync pull demo   # both start from the same baseline first
AURELIUS_HOME=$AURELIUS_HOME_B au sync pull demo
AURELIUS_HOME=$AURELIUS_HOME_A au note-update <node-id> "owner's edit"
AURELIUS_HOME=$AURELIUS_HOME_B au note-update <node-id> "tester's edit"
AURELIUS_HOME=$AURELIUS_HOME_A au sync push demo
AURELIUS_HOME=$AURELIUS_HOME_B au sync push demo   # loses the race, gets a 200 with conflicts:1
AURELIUS_HOME=$AURELIUS_HOME_A au sync pull demo
AURELIUS_HOME=$AURELIUS_HOME_A au context demo -v   # winning note visible; loser visible under data._sync_conflict
```

## 9. Non-shared projects stay private (FR-002, SC-006)

```bash
AURELIUS_HOME=$AURELIUS_HOME_A au note "unrelated private thought" -p personal
AURELIUS_HOME=$AURELIUS_HOME_A au sync push demo
AURELIUS_HOME=$AURELIUS_HOME_B au sync pull demo
AURELIUS_HOME=$AURELIUS_HOME_B au context personal   # empty — never shared, never sent
```

## 10. Server unreachable doesn't block local work (FR-006, FR-011, SC-005)

```bash
# stop the sync server process, then:
AURELIUS_HOME=$AURELIUS_HOME_A au note "still works offline" -p demo
AURELIUS_HOME=$AURELIUS_HOME_A au sync push demo   # warns, does not fail the shell exit code
```

See `contracts/sync-api.md` for the exact request/response shapes and
`data-model.md` for the schema backing each step above.
