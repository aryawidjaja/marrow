# Marrow backbone: one brain across devices

`marrow-server` is a shared Marrow store over HTTP. Point every device's agent at it and they read
and write the same memory. It speaks the same tool calls the local MCP server does, routed to a
per-project store on the server. This is the "from anywhere" setup (HTTPS, no shared network needed).

## 1. Deploy the backbone (Fly.io)

You need the repo (for `deploy/`) and the Fly CLI once:

```sh
brew install flyctl                 # or: curl -L https://fly.io/install.sh | sh
fly auth login                      # or `fly auth signup`
git clone https://github.com/aryawidjaja/marrow
cd marrow
```

Then create and deploy the app (the Dockerfile builds the backbone straight from GitHub, so this
folder is all Fly needs):

```sh
fly launch --no-deploy --copy-config -c deploy/fly.toml
fly secrets set MARROW_TOKEN=$(openssl rand -hex 32)   # your shared key. SAVE IT (password manager)
fly volumes create marrow_data --size 1       # persistent storage for the memories
fly deploy -c deploy/fly.toml
```

Fly gives you an HTTPS URL like `https://your-app.fly.dev`. Check it:

```sh
curl -s https://your-app.fly.dev/health        # {"ok":true,...}
```

Any Docker host works the same way (Render, Railway, a VPS): build `deploy/Dockerfile` from the
repository root, set
`MARROW_TOKEN`, mount a volume at `/data`, expose `8787`, terminate TLS at the platform. To run it
locally instead: `MARROW_TOKEN=$(openssl rand -hex 32) docker compose -f deploy/docker-compose.yml up`.

## 2. Share a project (on each device)

Each project on your machine is local and private by default. You share the *one* project you want
synced. Everything else stays put. In that project, on **every** machine:

```bash
MARROW_TOKEN=<the-token> marrow share --gateway https://your-app.fly.dev --space team-app
```

The rule: **same gateway + same space + same token = one brain.** `--space` is any label you pick,
as long as it matches on both machines. Then start a fresh agent session.

You can do the same from the dashboard: `marrow-serve`, open **Manage Projects**, hit **share**.

Test it: on machine A tell an agent in that project "remember: we deploy on Fly in sjc". On machine B,
ask a fresh agent in the same project "what did we decide about deploy?" It answers from the gateway.

## Does this delete my local memories?

**No. Nothing local is touched.** Sharing only changes where that *one* project reads and writes.
Its `.marrow/` folder stays exactly as it is, and every other project is untouched.

While shared, the agent talks to the gateway (which starts empty) and both machines build one brain
there together. Unshare and the agent goes straight back to the local store, with everything still in
it.

Want an existing project's memories *in* the shared space? Copy its markdown across on the gateway
host and reindex once:

```bash
cp -R ~/your-project/.marrow/memory/*  <data-dir>/team-app/.marrow/memory/
marrow --root <data-dir>/team-app doctor
```

## Go back to local

In the project:

```bash
marrow unshare      # local and private again; nothing is deleted
marrow status       # confirms: shared or local
```

## Machine-wide remote (the old way)

`MARROW_REMOTE` / `MARROW_TOKEN` / `MARROW_PROJECT` still work and send **every** project to one
backbone. Per-project `marrow share` is preferred: it's the difference between sharing one repo and
sharing your whole disk.

## See the shared brain in a dashboard

The backbone keeps each project under its data dir, so point the dashboard at it:

```sh
# on the backbone host (Fly: `fly ssh console`, or `fly ssh sftp` the folder down)
marrow-serve --root <data-dir>/shared-brain --port 8088   # open http://localhost:8088
```

## API

| Method | Path | Body | Purpose |
| --- | --- | --- | --- |
| GET | `/health` | | liveness (open, no token) |
| POST | `/v1/rpc` | `{"tool","args","project"}` | run a Marrow tool against a project store |

`Authorization: Bearer <MARROW_TOKEN>` is required on every request but `/health`.

## Scope and status

This is a beta backbone: auth is a single shared bearer token and isolation is per-project directory.
It's the foundation for the hosted/team edition (per-tenant API keys, quotas, backups), which is
roadmap. Run it on infrastructure you control, always over HTTPS for anything off your own machine,
and back up the mounted data volume. The local dashboard does not yet proxy the remote store; use it
to configure sharing and use MCP tools to read or write the shared brain. Code anchors and freshness
checks remain local-only because the backbone cannot see a client machine's source tree.
