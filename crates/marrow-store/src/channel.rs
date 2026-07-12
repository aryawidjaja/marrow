//! The agent channel: a lightweight, auditable way for agent sessions to talk — ask a question,
//! reply, see what's addressed to you. Messages are ordinary hash-chained ledger events, so nothing
//! is destructive and the human can read the whole conversation. Written to the shared `core` store
//! when a hive exists, so an agent in one project can reach an agent in another.

use serde_json::json;
use ulid::Ulid;

use crate::store::{Error, Store};

/// One message in a thread.
pub struct Message {
    pub thread: String,
    pub from: String,
    pub to: String,
    pub role: String,
    pub body: String,
    pub ts: String,
}

fn short(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= n {
        t.to_string()
    } else {
        format!("{}…", t.chars().take(n).collect::<String>().trim_end())
    }
}

impl Store {
    /// Post a message. `to` is a free-form target — a session id, an agent name, a project, or
    /// "all". Pass `thread` to reply within an existing thread, or `None` to start one. Returns the
    /// thread id.
    pub fn post_message(
        &self,
        from: &str,
        to: &str,
        thread: Option<&str>,
        role: &str,
        body: &str,
    ) -> Result<String, Error> {
        let thread = thread
            .map(str::to_string)
            .unwrap_or_else(|| Ulid::new().to_string());
        let summary = format!("{role} → {to}: {}", short(body, 60));
        self.log_data(
            "message",
            from,
            &summary,
            json!({ "to": to, "thread": thread, "role": role, "body": body }),
        )?;
        Ok(thread)
    }

    /// The latest message of each thread addressed to one of `me` (or "all") that `me` didn't send
    /// — i.e. what's waiting for this agent. Newest first, capped at `limit`.
    pub fn inbox(&self, me: &[String], limit: usize) -> Result<Vec<Message>, Error> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        let is_me = |s: &str| me.iter().any(|m| m == s);
        for e in self.messages()? {
            if !seen.insert(e.thread.clone()) {
                continue; // only the latest message per thread
            }
            if (e.to == "all" || is_me(&e.to)) && !is_me(&e.from) {
                out.push(e);
                if out.len() >= limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    /// Every message in a thread, oldest first.
    pub fn thread(&self, thread: &str) -> Result<Vec<Message>, Error> {
        let mut msgs: Vec<Message> = self
            .messages()?
            .into_iter()
            .filter(|m| m.thread == thread)
            .collect();
        msgs.reverse();
        Ok(msgs)
    }

    /// All messages, newest first.
    fn messages(&self) -> Result<Vec<Message>, Error> {
        let mut all = self.history()?;
        all.reverse();
        Ok(all
            .into_iter()
            .filter(|e| e.kind == "message")
            .map(|e| Message {
                thread: e
                    .data
                    .get("thread")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                from: e.actor,
                to: e
                    .data
                    .get("to")
                    .and_then(|v| v.as_str())
                    .unwrap_or("all")
                    .to_string(),
                role: e
                    .data
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("ask")
                    .to_string(),
                body: e
                    .data
                    .get("body")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&e.summary)
                    .to_string(),
                ts: e.ts,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_reply_and_inbox_route_by_recipient() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        let thread = store
            .post_message("alice", "bob", None, "ask", "how's the model training?")
            .unwrap();

        // Bob sees it; Carol does not.
        let bob = store.inbox(&["bob".into()], 10).unwrap();
        assert_eq!(bob.len(), 1);
        assert_eq!(bob[0].from, "alice");
        assert!(store.inbox(&["carol".into()], 10).unwrap().is_empty());

        // Bob replies; now the latest in the thread is from bob, so it's waiting for alice.
        store
            .post_message("bob", "alice", Some(&thread), "reply", "recall is 0.71")
            .unwrap();
        assert!(
            store.inbox(&["bob".into()], 10).unwrap().is_empty(),
            "bob shouldn't see his own reply"
        );
        let alice = store.inbox(&["alice".into()], 10).unwrap();
        assert_eq!(alice.len(), 1);
        assert_eq!(alice[0].body, "recall is 0.71");
        assert_eq!(store.thread(&thread).unwrap().len(), 2);
    }
}
