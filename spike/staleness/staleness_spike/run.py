from __future__ import annotations

import argparse
from pathlib import Path

from staleness_spike.report import write_reports
from staleness_spike.runner import run_spike


def main() -> None:
    parser = argparse.ArgumentParser(description="Run the staleness spike.")
    parser.add_argument("--corpus", default="corpus", help="path to vendored repo")
    parser.add_argument("--count", type=int, default=150, help="number of memories to seed")
    parser.add_argument("--seed", type=int, default=1234, help="random seed")
    parser.add_argument("--out", default="results", help="output directory")
    args = parser.parse_args()

    corpus = Path(args.corpus)
    if not corpus.exists():
        raise SystemExit(f"corpus not found at {corpus}; run ./fetch_corpus.sh first")

    rows = run_spike(corpus, count=args.count, seed=args.seed)
    write_reports(rows, Path(args.out))
    print(f"Wrote {len(rows)} result rows to {args.out}/ (scoreboard.md, raw.jsonl)")


if __name__ == "__main__":
    main()
