# Project Sync

[Back to README](../README.md)

Share a single project between two Aurelius instances — an owner and one or
more collaborators — over a self-hosted sync server, so new decisions, tasks,
and work log entries made on either side show up on the other at the next
session, attributed to who made them.

```
[Owner instance]  <--push/pull-->  [aurelius-sync-server]  <--push/pull-->  [Collaborator]
```

## Contents

- [How it works](#how-it-works)
- [Sync Commands](#sync-commands)
- [Self-hosting](#self-hosting)

## How it works

- **Bootstrap once, per machine:** `au identity set` configures who you are
  (stamped as `"Name <email>"` on everything you create/update). The owner
  then issues each collaborator a one-time token (`au share issue`); the
  collaborator connects with a single command, `au share <server> <token>` —
  no manual export/import, and their instance receives the project's full
  existing history.
- **Automatic thereafter:** `memory_status` pulls and `memory_session` pushes
  for any sync-enabled project; `au share push`/`pull` do the same from the
  CLI. Both are best-effort — an unreachable server never blocks local work.
- **Deletions and conflicts are safe:** deletions propagate as tombstones
  (never resurrected on a later sync); a same-record conflict resolves
  deterministically last-writer-wins, with the losing edit retained under
  `data._sync_conflict` for recovery (`au context <project> -v`).

## Sync Commands

Project sync (`identity`/`share`, below) — not the removed `au sync` TimeForged connector
(see the [CLI section's removal note](cli.md#command-overview)).

```bash
au identity set --name "Name" --email you@example.com          # once per machine

au share issue <project> --for "Name <email>" --server <host>   # owner: mint a token
au share revoke <project> --for <email> --server <host>         # owner: revoke access

au share <server> <token>          # collaborator: bootstrap + connect (once per project)
au share push [project]            # push local changes (default: every enabled project)
au share pull [project]            # pull remote changes (default: every enabled project)
au share list                      # show connected projects
au share disable <project>         # stop syncing (local data kept)
```

## Self-hosting

See [`deploy/aurelius-sync-server/README.md`](../deploy/aurelius-sync-server/README.md)
to self-host a sync server.
