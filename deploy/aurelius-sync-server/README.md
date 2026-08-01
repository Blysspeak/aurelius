# aurelius-sync-server — self-hosted deployment

This directory is a copy-pasteable deployment unit for `aurelius-sync-server`,
the hub-and-spoke HTTP endpoint that lets two or more Aurelius instances
two-way sync a shared project (see `specs/002-project-sync/`). Copy this
directory (or just clone the whole repo) and run your own private instance —
no code changes required.

## Prerequisites

- Docker + the `docker compose` plugin.
- A place to run it with a stable reachable address (a VPS, home server,
  etc.) — the server itself has no cloud dependency.
- Optional but recommended: a reverse proxy in front of it for TLS (see
  below). The server itself speaks plain HTTP.

## 1. Configure

```bash
cp deploy/aurelius-sync-server/.env.example deploy/aurelius-sync-server/.env
```

Edit `.env` and set `AURELIUS_SYNC_ADMIN_TOKEN` — this guards the
admin-only `POST /sync/grants` and `/sync/grants/revoke` endpoints (issuing
and revoking collaborator access). Generate one with:

```bash
openssl rand -hex 32
```

`AURELIUS_SYNC_PORT` (default `8181`) is the only other setting; change it
if that host port is already taken.

## 2. Run

From the repository root:

```bash
docker compose -f deploy/aurelius-sync-server/docker-compose.yml up -d --build
```

This builds the `aurelius-sync-server` binary from the workspace (the build
context is pinned to the repo root — `aurelius-sync-server` depends on
`aurelius-core` via a path dependency, so the whole workspace tree is needed
to resolve it) and starts the container. The server's SQLite database lives
on a named Docker volume (`aurelius-sync-data`), so it survives container
recreation and image rebuilds.

Check it's up:

```bash
docker compose -f deploy/aurelius-sync-server/docker-compose.yml logs -f
```

You should see `aurelius-sync-server listening` with the bound address.

## 3. Grant a collaborator access

Access is per-project, per-person, and always issued by the owner from
their own `au` CLI — there's no self-service enrollment. The target project
must already exist locally (`au` looks it up by label; it will not silently
create one for you if you typo the name).

```bash
AURELIUS_SYNC_ADMIN_TOKEN=<your admin token> \
  au share issue <project> --for "Tester Name <tester@example.com>" --server <your-host-or-url>
```

This prints a one-time token. Hand it to the collaborator out of band (chat,
password manager, etc. — never over the sync channel itself). They connect
their own local project with:

```bash
au share <your-host-or-url> <token>
```

That single command bootstraps their instance with the project's full
existing history and enables ongoing sync — the project name itself is
learned from the server's response, not typed by the collaborator.

To revoke access later:

```bash
AURELIUS_SYNC_ADMIN_TOKEN=<your admin token> \
  au share revoke <project> --for tester@example.com --server <your-host-or-url>
```

Revocation stops future push/pull for that credential; it does not retract
data already delivered to that collaborator's instance.

`<your-host-or-url>` can be a bare host (`sync.example.com`, normalized to
`https://sync.example.com/sync`) or a full URL, including `http://` for
local testing against `localhost`.

## Reverse proxy

`aurelius-sync-server` only speaks plain HTTP on the port it's given — it
has no built-in TLS. Put it behind any TLS-terminating reverse proxy
(Caddy, nginx, Traefik, a cloud load balancer, etc.), proxying to
`http://127.0.0.1:${AURELIUS_SYNC_PORT}` and forwarding the path prefix the
server expects requests under: `/sync/push`, `/sync/pull`, `/sync/grants`,
`/sync/grants/revoke`. There's nothing domain- or provider-specific about
this — any host that terminates TLS and forwards to the container's port
works.

## Upgrading

```bash
git pull
docker compose -f deploy/aurelius-sync-server/docker-compose.yml up -d --build
```

The database volume is untouched by rebuilds; schema migrations run
automatically on server startup (same additive-migration mechanism as the
rest of Aurelius).

---

### This project's own deployment

*Documentation only — not something to act on from this checklist.*

This repository's own instance runs on the owner's `boostix` VPS, reverse-proxied
at `aurelius.boostix.space/sync`. Deploying or changing that specific instance
(SSH access, DNS, the real production admin token) is a shared-infrastructure
change the owner does by hand, not something automated from this repo.
