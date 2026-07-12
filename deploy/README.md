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
cd marrow/deploy
```

Then create and deploy the app (the Dockerfile builds the backbone straight from GitHub, so this
folder is all Fly needs):

```sh
fly launch --no-deploy --copy-config          # accept the app name/region it proposes
fly secrets set MARROW_TOKEN=$(openssl rand -hex 32)   # your shared key. SAVE IT (password manager)
fly volumes create marrow_data --size 1       # persistent storage for the memories
fly deploy
```

Fly gives you an HTTPS URL like `https://your-app.fly.dev`. Check it:

```sh
curl -s https://your-app.fly.dev/health        # {"ok":true,...}
```

Any Docker host works the same way (Render, Railway, a VPS): build `deploy/Dockerfile`, set
`MARROW_TOKEN`, mount a volume at `/data`, expose `8787`, terminate TLS at the platform. To run it
locally instead: `MARROW_TOKEN=$(openssl rand -hex 32) docker compose -f deploy/docker-compose.yml up`.

## 2. Connect each device

On **every** machine (do this once per machine), point Claude Code's agent at the backbone. The
cleanest way is to bake the connection into the MCP registration, so it works no matter how you
launch Claude Code:

```sh
marrow setup --global               # once, if you haven't (installs hooks)

claude mcp remove marrow -s user 2>/dev/null
claude mcp add marrow -s user \
  -e MARROW_REMOTE=https://your-app.fly.dev \
  -e MARROW_TOKEN=your-shared-key \
  -e MARROW_PROJECT=shared-brain \
  -- marrow-mcp --root .
```

- `MARROW_TOKEN` must be the **same** on every device (the one from `fly secrets set`).
- `MARROW_PROJECT` must be the **same** on every device (any name; same name = same shared brain).
- Then **fully quit and reopen Claude Code**.

Test: on machine A tell an agent "remember: we deploy on Fly in sjc." On machine B, ask a fresh
agent "what did we decide about deploy?" It answers from the shared backbone.

## Does this delete my local memories?

**No. Nothing local is touched or lost.** Turning on `MARROW_REMOTE` only *redirects* where the
agent reads and writes. Your existing `.marrow/` folders stay on disk exactly as they are.

- While connected, the agent talks to the **backbone**, which starts empty. Both devices then build
  up one shared brain together. Your old per-project local memories are simply not shown during this
  time (they live on your disk, not on the backbone).
- Disconnect (below) and the agent goes back to your local store with every local memory still there.

Want to bring an existing project's memories *into* the shared brain? Copy its markdown over on the
backbone host and reindex once:

```sh
# on the backbone host (or into the Fly volume via `fly ssh console`)
cp -R ~/your-project/.marrow/memory/*  <data-dir>/shared-brain/.marrow/memory/
marrow --root <data-dir>/shared-brain doctor      # rebuild the index from the files
```

## Disconnect / go back to local

To take a device back off the hive and use its own local memory again:

```sh
marrow setup --global               # re-registers Marrow with no remote, so it's local again
# then restart Claude Code
```

That's it. Nothing is deleted either way; you're just switching which brain the agent talks to. (Or,
if you prefer, `claude mcp remove marrow -s user` then re-add without the `-e` lines.)

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
roadmap. Run it on infrastructure you control, always over HTTPS for anything off your own machine.
