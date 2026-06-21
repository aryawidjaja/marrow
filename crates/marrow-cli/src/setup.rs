//! `marrow setup` — wire Marrow into Claude Code for a project in one command.
//!
//! It registers the MCP server (user scope, so every project gets the tools), installs the
//! auto-capture hooks, and drops a short guidance block into `CLAUDE.md`. The hook scripts are
//! embedded in the binary, so this needs no cloned repo. After running it and restarting Claude
//! Code, sessions warm-start, avoid file collisions, and capture decisions automatically.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;

const BOOTSTRAP: &str = include_str!("../../../integrations/claude-code/hooks/marrow-bootstrap.sh");
const GUARD: &str = include_str!("../../../integrations/claude-code/hooks/marrow-guard.sh");
const PROGRESS: &str = include_str!("../../../integrations/claude-code/hooks/marrow-progress.sh");
const SETTINGS: &str = include_str!("../../../integrations/claude-code/settings.example.json");

const GUIDANCE: &str = "<!-- marrow:begin (managed by `marrow setup`) -->\n\
## Marrow shared memory\n\n\
This project has a Marrow shared brain connected over MCP. Hooks bootstrap context at session\n\
start, prevent file collisions before edits, and record activity — automatically. You only need\n\
to do one thing: when you reach a durable decision, fact, or gotcha, save it with the `mem_write`\n\
tool (kind `decision`/`fact`, a short topic). Use `mem_recall` before answering questions about\n\
past decisions, and don't re-save anything already in Marrow.\n\
<!-- marrow:end -->\n";

/// Add the marrow binary's own directory to a hook's lookup chain, so the hook finds `marrow`
/// regardless of how it was installed (brew, cargo, curl) or the hook shell's PATH.
fn with_bin_dir(hook: &str, bin_dir: &str) -> String {
    if bin_dir.is_empty() {
        return hook.to_string();
    }
    let needle = "marrow=\"$(command -v marrow || true)\"";
    let injected = format!(
        "{needle}\n[ -z \"$marrow\" ] && [ -x \"{bin_dir}/marrow\" ] && marrow=\"{bin_dir}/marrow\""
    );
    hook.replacen(needle, &injected, 1)
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
}
#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

/// Run `marrow setup` against `root`.
pub fn run(root: &Path, out: &mut impl Write) -> Result<(), String> {
    let bin_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();

    // 1) Make sure the store exists (it would auto-create on first use anyway).
    let _ = marrow_store::Store::init(root);

    // 2) Install the auto-capture hooks (with the binary location baked into their lookup).
    let hooks_dir = root.join(".claude").join("hooks");
    fs::create_dir_all(&hooks_dir).map_err(|e| e.to_string())?;
    for (name, body) in [
        ("marrow-bootstrap.sh", BOOTSTRAP),
        ("marrow-guard.sh", GUARD),
        ("marrow-progress.sh", PROGRESS),
    ] {
        let path = hooks_dir.join(name);
        fs::write(&path, with_bin_dir(body, &bin_dir)).map_err(|e| e.to_string())?;
        make_executable(&path);
    }
    writeln!(
        out,
        "  hooks       -> .claude/hooks/ (bootstrap, guard, progress)"
    )
    .ok();

    // 3) Register the hooks in settings.json without clobbering an existing one.
    let settings = root.join(".claude").join("settings.json");
    if settings.exists() {
        fs::write(root.join(".claude").join("settings.marrow.json"), SETTINGS)
            .map_err(|e| e.to_string())?;
        writeln!(
            out,
            "  settings    -> .claude/settings.json exists; wrote settings.marrow.json (merge its \"hooks\" block)"
        )
        .ok();
    } else {
        fs::write(&settings, SETTINGS).map_err(|e| e.to_string())?;
        writeln!(out, "  settings    -> .claude/settings.json").ok();
    }

    // 4) Add the guidance block to CLAUDE.md (idempotent).
    let claude_md = root.join("CLAUDE.md");
    let has_block = fs::read_to_string(&claude_md)
        .map(|c| c.contains("marrow:begin"))
        .unwrap_or(false);
    if has_block {
        writeln!(
            out,
            "  guidance    -> CLAUDE.md already has the Marrow block"
        )
        .ok();
    } else {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&claude_md)
            .map_err(|e| e.to_string())?;
        write!(f, "\n{GUIDANCE}").map_err(|e| e.to_string())?;
        writeln!(out, "  guidance    -> added the Marrow block to CLAUDE.md").ok();
    }

    // 5) Register the MCP server at user scope so every project gets the tools.
    let mcp_bin = if bin_dir.is_empty() {
        "marrow-mcp".to_string()
    } else {
        format!("{bin_dir}/marrow-mcp")
    };
    match Command::new("claude")
        .args([
            "mcp", "add", "marrow", "-s", "user", "--", &mcp_bin, "--root", ".",
        ])
        .output()
    {
        Ok(o) if o.status.success() => {
            writeln!(
                out,
                "  mcp         -> registered at user scope (available in every project)"
            )
            .ok();
        }
        Ok(_) => {
            writeln!(out, "  mcp         -> already registered at user scope").ok();
        }
        Err(_) => {
            writeln!(
                out,
                "  mcp         -> claude CLI not found; register manually:\n                 claude mcp add marrow -s user -- marrow-mcp --root ."
            )
            .ok();
        }
    }

    writeln!(
        out,
        "\nDone. Restart Claude Code in this project — it will warm-start, avoid collisions, and capture decisions automatically."
    )
    .ok();
    Ok(())
}
