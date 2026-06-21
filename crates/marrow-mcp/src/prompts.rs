//! MCP prompt templates. Clients surface these as reusable commands (Claude Code shows them as
//! `/mcp__marrow__<name>`), so a user can trigger them without typing the instruction by hand.

use serde_json::{json, Value};

const SAVE: &str =
    "Review our conversation so far and save what's worth remembering to the Marrow \
shared brain, so the next session inherits it. For each durable decision, fact, or gotcha: call \
mem_recall first and skip anything already stored, then save it with mem_write (kind `decision` or \
`fact`, a short topic). Distill — capture the conclusion and the why, not the transcript. When \
done, briefly list what you saved.";

/// The prompt catalog advertised via `prompts/list`.
pub fn definitions() -> Value {
    json!([
        {
            "name": "save",
            "description": "Save this session's durable decisions and facts to the Marrow shared brain.",
            "arguments": []
        }
    ])
}

/// The full prompt for `prompts/get`, or `None` if the name is unknown.
pub fn get(name: &str) -> Option<Value> {
    match name {
        "save" => Some(json!({
            "description": "Save this session's durable decisions and facts to the Marrow shared brain.",
            "messages": [
                {"role": "user", "content": {"type": "text", "text": SAVE}}
            ]
        })),
        _ => None,
    }
}
