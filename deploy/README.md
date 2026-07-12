# Marrow backbone, one brain across devices

`marrow-server` is a shared Marrow store over HTTP. Point every device's agent at it and they read
and write the same memory, the hive-mind, across machines. It speaks the same tool calls the local
MCP server does, routed to a per-project store on the server.

## Run it

**Locally (Docker):**

```sh
export MARROW_TOKEN=$(openssl rand -hex 16)
docker compose -f deploy/docker-compose.yml up --build
# backbone on http://localhost:8787, data in the marrow-data volume
```

**Fly.io:**

```sh
cd deploy
fly apps create marrow-backbone
fly secrets set MARROW_TOKEN=$(openssl rand -hex 16)
fly volumes create marrow_data --size 1 --region sjc
fly deploy
```

Any Docker host (Render, Railway, a VPS) works the same way: build `deploy/Dockerfile`, set
`MARROW_TOKEN`, mount a volume at `/data`, expose `8787`. Terminate TLS at the platform's proxy.

## Connect a device

Set three environment variables where the agent runs, and its Marrow tools transparently hit the
backbone instead of a local store:

```sh
export MARROW_REMOTE=https://marrow-backbone.fly.dev
export MARROW_TOKEN=<the shared secret>
export MARROW_PROJECT=team-app        # devices sharing this name share one brain
```

That's it, `mem_write`, `mem_recall`, `mem_search`, and the rest now round-trip through the
shared backbone. Leave the variables unset and Marrow stays fully local.

## API

| Method | Path | Body | Purpose |
| --- | --- | --- | --- |
| GET | `/health` |, | liveness (open, no token) |
| POST | `/v1/rpc` | `{"tool","args","project"}` | run a Marrow tool against a project store |

`Authorization: Bearer <MARROW_TOKEN>` is required on every request but `/health`.

## Scope and status

This is a beta backbone: authentication is a single shared bearer token and isolation is
per-project directory. It is the foundation for the hosted/team edition (per-tenant auth, TLS,
quotas, backups), which is roadmap, not shipped. Run it on infrastructure you control.
