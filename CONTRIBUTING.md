# Contributing to Marrow

Thanks for your interest in Marrow. A few conventions keep the project healthy.

## Workflow: branch, then PR

**Do not commit directly to `main`.** Work happens on short-lived branches and lands through pull
requests that a maintainer reviews and merges.

```bash
git switch -c feat/short-description     # or fix/…, docs/…
# make your change
git commit -m "concise one-line summary"
git push -u origin feat/short-description # then open a PR
```

This holds for *every* change, including small ones and changes made with the help of automated
tools, branch it, open a PR, let it be reviewed. It keeps `main` releasable and prevents
incidental edits from landing unintentionally.

## Keep it green

Before opening a PR, make sure the full gate passes locally, CI runs the same checks:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./e2e/smoke.sh
```

New behavior needs tests. We practice test-driven development: add a failing test, make it pass,
keep it.

## Commit messages

One concise line in the imperative mood, e.g. `fix: relocate moved code anchors`. Conventional
prefixes (`feat:`, `fix:`, `docs:`, `chore:`) are welcome. Keep them factual and free of noise.

## Code style

- Match the surrounding code; keep modules focused and files small.
- No dead code, no filler comments, every line should earn its place.
- Markdown is the source of truth in Marrow; the SQLite index is a rebuildable cache. Don't make
  the index authoritative.

## Scope

Keep a PR to one logical change. If you find an unrelated issue, note it or open a separate PR.
