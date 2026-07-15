# Staleness Spike

An experiment in detecting when a code-anchored note has gone stale. Each "memory"
cites a Python symbol; after the code is mutated, four anchoring strategies are scored
on whether they correctly tell a genuine change from a harmless one (reformatting,
renaming a local, moving a function).

## Strategies

- **s1_exact** — SHA-256 of the raw snippet at the exact line range (brittle baseline).
- **s2_normalized** — whitespace-normalized hash searched within the file.
- **s3_ast** — normalized-AST fingerprint of the symbol, located by name.
- **s4_relocation** — normalized content hash searched across the whole repo, with relocation.

## Run it

```bash
./fetch_corpus.sh                  # vendor the sample repo at a pinned commit into corpus/
python3 -m pip install -e ".[dev]"
python3 -m pytest                  # unit tests
python3 -m staleness_spike.run
```

Outputs `results/scoreboard.md` and `results/raw.jsonl`.

## Scoring

`stale` is the positive class. The experiment compares false positives, missed stale memories, and
the different refactor classes each strategy handles. These are engineering diagnostics, not public
product claims.

## Reproducibility

The corpus is pinned (`corpus/.pinned-commit`) and seeding/mutation selection use the
`--seed` value, so the same corpus and seed produce an identical scoreboard.
