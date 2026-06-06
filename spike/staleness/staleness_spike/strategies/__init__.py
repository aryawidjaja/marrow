from __future__ import annotations

from staleness_spike.strategies.base import Strategy

# Populated as each strategy module is added (Tasks 5-8).
STRATEGIES: list[Strategy] = []


def register(strategy: type[Strategy]) -> type[Strategy]:
    STRATEGIES.append(strategy())  # store an instance; decorator still returns the class
    return strategy
