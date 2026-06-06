//! Core data types for code-anchored staleness detection.

/// A memory anchored to a code symbol, captured at seed time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    /// Path to the file containing the symbol, relative to the repo root (posix separators).
    pub file_path: String,
    /// Qualified symbol name, e.g. `"Foo::bar"`.
    pub symbol: String,
    /// Raw source text of the symbol at seed time.
    pub snippet: String,
    /// Structural fingerprint (S3) of the symbol, hex-encoded.
    pub fingerprint: String,
    /// Whitespace-normalized snippet text (S4 needle).
    pub norm: String,
}

/// The result of checking an anchor against the current repo state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// Whether the memory is stale (the cited code changed or vanished).
    pub stale: bool,
    /// Where the symbol was relocated to (`"path:line"`), if a cross-file move was detected.
    pub relocated_to: Option<String>,
}
