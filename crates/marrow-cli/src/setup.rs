//! `marrow setup` — wire Marrow into Claude Code for a project in one command.
//!
//! It registers the MCP server (user scope, so every project gets the tools), installs the
//! auto-capture hooks, and drops a short guidance block into `CLAUDE.md`. The hook scripts are
//! embedded in the binary, so this needs no cloned repo. After running it and restarting Claude
//! Code, sessions warm-start, avoid file collisions, and capture decisions automatically.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const BOOTSTRAP: &str = include_str!("../../../integrations/claude-code/hooks/marrow-bootstrap.sh");
const GUARD: &str = include_str!("../../../integrations/claude-code/hooks/marrow-guard.sh");
const PROGRESS: &str = include_str!("../../../integrations/claude-code/hooks/marrow-progress.sh");
const SETTINGS: &str = include_str!("../../../integrations/claude-code/settings.example.json");
const MARROW_SAVE: &str = include_str!("../../../integrations/claude-code/commands/marrow-save.md");

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

fn current_bin_dir() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Run `marrow setup`. Without `global`, it wires this project's `.claude`; with `global`, it wires
/// the user-level `~/.claude` so every project is hands-free.
pub fn run(root: &Path, global: bool, out: &mut impl Write) -> Result<(), String> {
    let bin_dir = current_bin_dir();

    // Where Claude Code config lives, and which CLAUDE.md gets the guidance block.
    let (base, claude_md, init_root, label) = if global {
        let base = home_dir()
            .ok_or("could not determine home directory for --global")?
            .join(".claude");
        let claude_md = base.join("CLAUDE.md");
        (base, claude_md, None, "~/.claude".to_string())
    } else {
        (
            root.join(".claude"),
            root.join("CLAUDE.md"),
            Some(root.to_path_buf()),
            ".claude".to_string(),
        )
    };

    install(
        &base,
        &claude_md,
        init_root.as_deref(),
        &bin_dir,
        &label,
        out,
    )?;
    register_mcp(&bin_dir, out);

    writeln!(
        out,
        "\nDone. Restart Claude Code — it will warm-start, avoid collisions, and capture decisions automatically."
    )
    .ok();
    Ok(())
}

/// Install the file-based pieces (hooks, settings, slash command, guidance) under `base`. Pure
/// filesystem work, so it's unit-testable without shelling out to the `claude` CLI.
fn install(
    base: &Path,
    claude_md: &Path,
    init_root: Option<&Path>,
    bin_dir: &str,
    label: &str,
    out: &mut impl Write,
) -> Result<(), String> {
    // 1) Make sure a project store exists (global setup has no single project; stores auto-create).
    if let Some(root) = init_root {
        let _ = marrow_store::Store::init(root);
    }

    // 2) Install the auto-capture hooks (with the binary location baked into their lookup).
    let hooks_dir = base.join("hooks");
    fs::create_dir_all(&hooks_dir).map_err(|e| e.to_string())?;
    for (name, body) in [
        ("marrow-bootstrap.sh", BOOTSTRAP),
        ("marrow-guard.sh", GUARD),
        ("marrow-progress.sh", PROGRESS),
    ] {
        let path = hooks_dir.join(name);
        fs::write(&path, with_bin_dir(body, bin_dir)).map_err(|e| e.to_string())?;
        make_executable(&path);
    }
    writeln!(
        out,
        "  hooks       -> {label}/hooks/ (bootstrap, guard, progress)"
    )
    .ok();

    // 3) Register the hooks in settings.json without clobbering an existing one.
    let settings = base.join("settings.json");
    if settings.exists() {
        fs::write(base.join("settings.marrow.json"), SETTINGS).map_err(|e| e.to_string())?;
        writeln!(
            out,
            "  settings    -> {label}/settings.json exists; wrote settings.marrow.json (merge its \"hooks\" block)"
        )
        .ok();
    } else {
        fs::write(&settings, SETTINGS).map_err(|e| e.to_string())?;
        writeln!(out, "  settings    -> {label}/settings.json").ok();
    }

    // 4) Drop the /marrow-save slash command.
    let commands_dir = base.join("commands");
    fs::create_dir_all(&commands_dir).map_err(|e| e.to_string())?;
    fs::write(commands_dir.join("marrow-save.md"), MARROW_SAVE).map_err(|e| e.to_string())?;
    writeln!(
        out,
        "  command     -> {label}/commands/marrow-save.md (/marrow-save)"
    )
    .ok();

    // 5) Add the guidance block to CLAUDE.md (idempotent).
    let has_block = fs::read_to_string(claude_md)
        .map(|c| c.contains("marrow:begin"))
        .unwrap_or(false);
    if has_block {
        writeln!(
            out,
            "  guidance    -> CLAUDE.md already has the Marrow block"
        )
        .ok();
    } else {
        if let Some(parent) = claude_md.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(claude_md)
            .map_err(|e| e.to_string())?;
        write!(f, "\n{GUIDANCE}").map_err(|e| e.to_string())?;
        writeln!(out, "  guidance    -> added the Marrow block to CLAUDE.md").ok();
    }

    Ok(())
}

/// Register the MCP server at user scope so every project gets the tools.
fn register_mcp(bin_dir: &str, out: &mut impl Write) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_writes_hooks_settings_command_and_guidance() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join(".claude");
        let claude_md = dir.path().join("CLAUDE.md");
        let mut out = Vec::new();

        install(&base, &claude_md, Some(dir.path()), "", ".claude", &mut out).unwrap();

        assert!(base.join("hooks/marrow-bootstrap.sh").exists());
        assert!(base.join("hooks/marrow-guard.sh").exists());
        assert!(base.join("hooks/marrow-progress.sh").exists());
        assert!(base.join("settings.json").exists());
        assert!(base.join("commands/marrow-save.md").exists());
        assert!(fs::read_to_string(&claude_md)
            .unwrap()
            .contains("marrow:begin"));

        // Idempotent: a second run doesn't duplicate the guidance block.
        install(&base, &claude_md, Some(dir.path()), "", ".claude", &mut out).unwrap();
        let body = fs::read_to_string(&claude_md).unwrap();
        assert_eq!(body.matches("marrow:begin").count(), 1);
    }

    #[test]
    fn install_global_style_creates_user_level_claude_md() {
        // Mimics --global: base is a ~/.claude-like dir with no project store.
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("dot-claude");
        let claude_md = base.join("CLAUDE.md");
        let mut out = Vec::new();

        install(&base, &claude_md, None, "", "~/.claude", &mut out).unwrap();

        assert!(base.join("hooks/marrow-bootstrap.sh").exists());
        assert!(base.join("commands/marrow-save.md").exists());
        assert!(fs::read_to_string(&claude_md)
            .unwrap()
            .contains("marrow:begin"));
    }
}
