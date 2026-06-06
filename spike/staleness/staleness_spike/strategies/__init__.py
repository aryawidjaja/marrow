from __future__ import annotations

from staleness_spike.strategies.base import Strategy

# Populated as each strategy module is added (Tasks 5-8).
STRATEGIES: list[Strategy] = []


def register(strategy: Strategy) -> Strategy:
    STRATEGIES.append(strategy)
    return strategy
