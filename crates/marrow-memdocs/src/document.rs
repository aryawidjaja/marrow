//! Parse and serialize memory documents (`---` YAML frontmatter + markdown body).

use crate::types::{Frontmatter, Memory};

/// Error from parsing a memory document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The document does not start with a `---` frontmatter fence.
    MissingFrontmatter,
    /// The closing `---` fence was not found.
    UnterminatedFrontmatter,
    /// The YAML frontmatter failed to deserialize (message included).
    Yaml(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::MissingFrontmatter => write!(f, "document does not begin with '---' frontmatter"),
            ParseError::UnterminatedFrontmatter => write!(f, "frontmatter '---' fence is not closed"),
            ParseError::Yaml(m) => write!(f, "invalid frontmatter: {m}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Parse a markdown document with leading YAML frontmatter into a [`Memory`].
pub fn parse(text: &str) -> Result<Memory, ParseError> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .ok_or(ParseError::MissingFrontmatter)?;
    let end = find_fence(rest).ok_or(ParseError::UnterminatedFrontmatter)?;
    let yaml = &rest[..end.0];
    let body = &rest[end.1..];
    let frontmatter: Frontmatter =
        serde_yaml::from_str(yaml).map_err(|e| ParseError::Yaml(e.to_string()))?;
    Ok(Memory {
        frontmatter,
        body: body.trim_start_matches(['\n', '\r']).to_string(),
    })
}

/// Find the closing `---` fence; returns (yaml_end, body_start) byte offsets within `rest`.
fn find_fence(rest: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" {
            return Some((offset, offset + line.len()));
        }
        offset += line.len();
    }
    None
}

/// Serialize a [`Memory`] back to `---` frontmatter + body markdown.
pub fn to_markdown(memory: &Memory) -> String {
    let yaml = serde_yaml::to_string(&memory.frontmatter).unwrap_or_default();
    let mut out = String::with_capacity(yaml.len() + memory.body.len() + 16);
    out.push_str("---\n");
    out.push_str(&yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n");
    if !memory.body.is_empty() {
        out.push('\n');
        out.push_str(&memory.body);
        if !memory.body.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MemoryKind, Status};

    const DOC: &str = "---\n\
id: 01ABC\n\
type: decision\n\
status: active\n\
topic: auth\n\
scope:\n  project_id: demo\n\
provenance:\n  written_by: agent-1\n\
created_at: 2026-06-06T00:00:00Z\n\
updated_at: 2026-06-06T00:00:00Z\n\
---\n\nWe use JWT for sessions.\n";

    #[test]
    fn parses_frontmatter_and_body() {
        let m = parse(DOC).expect("parse");
        assert_eq!(m.frontmatter.id, "01ABC");
        assert_eq!(m.frontmatter.kind, MemoryKind::Decision);
        assert_eq!(m.frontmatter.status, Status::Active);
        assert_eq!(m.frontmatter.scope.project_id, "demo");
        assert_eq!(m.frontmatter.confidence, 1.0); // defaulted
        assert_eq!(m.body, "We use JWT for sessions.\n");
    }

    #[test]
    fn missing_frontmatter_errors() {
        assert_eq!(parse("no fence here"), Err(ParseError::MissingFrontmatter));
    }

    #[test]
    fn unterminated_frontmatter_errors() {
        assert_eq!(parse("---\nid: x\n"), Err(ParseError::UnterminatedFrontmatter));
    }

    #[test]
    fn round_trips() {
        let m = parse(DOC).expect("parse");
        let text = to_markdown(&m);
        let m2 = parse(&text).expect("reparse");
        assert_eq!(m, m2);
    }
}
