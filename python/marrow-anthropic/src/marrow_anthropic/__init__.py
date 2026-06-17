"""A production-grade backend for Anthropic's memory tool (memory_20250818)."""

from .store import MemoryStore

__all__ = ["MemoryStore", "MarrowMemoryBackend"]


def __getattr__(name: str):
    # Expose the SDK-dependent backend lazily so the core has no hard dependency.
    if name == "MarrowMemoryBackend":
        from .backend import MarrowMemoryBackend

        return MarrowMemoryBackend
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
