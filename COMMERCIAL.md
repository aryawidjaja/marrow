# Licensing

Marrow is open source and dual-licensed.

## The open-source license

- **The Marrow engine and tools** (everything under `crates/`, including the store, CLI, MCP server,
  and dashboard) are licensed under the **GNU AGPL-3.0-only** (see [LICENSE](LICENSE)).
- **The Anthropic memory-tool backend** (`python/marrow-anthropic`), which you embed in your own
  application, is licensed under **Apache-2.0** (see [LICENSE-APACHE](LICENSE-APACHE)) so it never
  imposes copyleft on your code.

### Why AGPL, and why it does *not* affect normal use

AGPL keeps Marrow genuinely open and auditable (you can read every line and confirm nothing
leaves your infrastructure) while preventing anyone from taking Marrow, running it as a closed
hosted service, and giving nothing back.

Crucially, **using Marrow does not make your agent or application a derivative work.** Agents
talk to Marrow as a *separate process* over the Model Context Protocol (or the CLI). Calling a
separate program across an IPC boundary is not linking, so the AGPL's copyleft does not reach
your agent, your prompts, or your codebase. Run it, build on it, ship your product, the only
obligation AGPL creates is for someone who *modifies Marrow itself and offers that modified
Marrow to others over a network*, who must then share their Marrow changes.

## The commercial license (Marrow Sovereign)

If your organization cannot accept the AGPL's terms, a commercial license that lifts its
obligations is available. Managed access control, compliance exports, support, and SLA offerings
are roadmap items; contact us to discuss requirements rather than assuming they ship in the open
source backbone today.

Contact: Mutaqin Aryawijaya, mutaqin.aryawijaya@gmail.com
