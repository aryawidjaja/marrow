//! The agent channel: a lightweight, auditable way for agent sessions to talk — ask a question,
//! reply, see what's addressed to you. Messages are ordinary hash-chained ledger events, so nothing
//! is destructive and the human can read the whole conversation. Written to the shared `core` store
//! when a hive exists, so an agent in one project can reach an agent in another.

use std::collections::BTreeSet;

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
    /// The room's subject, carried by the message that opened the thread.
    pub topic: Option<String>,
}

/// A room: one conversation, on one subject, with everyone who has spoken in it.
pub struct Room {
    pub thread: String,
    pub topic: String,
    pub participants: Vec<String>,
    pub last_ts: String,
    pub messages: usize,
    pub unread: usize,
    pub last_from: String,
    pub last_body: String,
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
        self.post_to_room(from, to, thread, role, body, None)
    }

    /// Post a message, optionally opening a new room with a subject. A room is a thread with a
    /// `topic`; whoever replies is in it, so this is a group chat, not a pair of mailboxes.
    pub fn post_to_room(
        &self,
        from: &str,
        to: &str,
        thread: Option<&str>,
        role: &str,
        body: &str,
        topic: Option<&str>,
    ) -> Result<String, Error> {
        let thread = thread
            .map(str::to_string)
            .unwrap_or_else(|| Ulid::new().to_string());
        let subject = topic.map(str::trim).filter(|t| !t.is_empty());
        let summary = match subject {
            Some(t) => format!("{role} → {to} [{t}]: {}", short(body, 50)),
            None => format!("{role} → {to}: {}", short(body, 60)),
        };
        let mut data = json!({ "to": to, "thread": thread, "role": role, "body": body });
        if let Some(t) = subject {
            data["topic"] = json!(t);
        }
        self.log_data("message", from, &summary, data)?;
        Ok(thread)
    }

    /// Everything addressed to `me` (or "all") that `me` has not read and did not send, oldest
    /// first. Every message, not one-per-thread: showing only the newest would silently drop
    /// whatever was said before it. Call [`Store::mark_read`] to catch up.
    pub fn unread(&self, me: &[String], limit: usize) -> Result<Vec<Message>, Error> {
        let watermark = self.read_watermark(me)?;
        let is_me = |s: &str| me.iter().any(|m| m == s);
        let topics = self.thread_topics()?;
        let mut out: Vec<Message> = self
            .messages()?
            .into_iter()
            .filter(|m| (m.to == "all" || is_me(&m.to)) && !is_me(&m.from))
            .filter(|m| watermark.as_deref().is_none_or(|w| m.ts.as_str() > w))
            .map(|mut m| {
                m.topic = topics.get(&m.thread).cloned();
                m
            })
            .collect();
        out.reverse(); // messages() is newest-first; a conversation should read oldest-first
        if out.len() > limit {
            out.drain(..out.len() - limit); // keep the most recent `limit`
        }
        Ok(out)
    }

    /// The latest message of each thread addressed to one of `me` (or "all") that `me` didn't send.
    /// Newest first, capped at `limit`.
    pub fn inbox(&self, me: &[String], limit: usize) -> Result<Vec<Message>, Error> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        let is_me = |s: &str| me.iter().any(|m| m == s);
        let topics = self.thread_topics()?;
        for mut e in self.messages()? {
            if !seen.insert(e.thread.clone()) {
                continue; // only the latest message per thread
            }
            if (e.to == "all" || is_me(&e.to)) && !is_me(&e.from) {
                e.topic = topics.get(&e.thread).cloned();
                out.push(e);
                if out.len() >= limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    /// Mark everything up to now as read for `me`. Recorded as a ledger event, so read state is
    /// history like everything else rather than a flag someone can quietly flip.
    pub fn mark_read(&self, me: &str) -> Result<(), Error> {
        let now = crate::util::now_rfc3339();
        self.log_data(
            "channel_read",
            me,
            &format!("{me} caught up on the channel"),
            json!({ "agent": me, "until": now }),
        )
    }

    /// The newest point any of `me`'s names has caught up to.
    fn read_watermark(&self, me: &[String]) -> Result<Option<String>, Error> {
        let mut best: Option<String> = None;
        for e in self.history()? {
            if e.kind != "channel_read" {
                continue;
            }
            let agent = e.data["agent"].as_str().unwrap_or_default();
            if !me.iter().any(|m| m == agent) {
                continue;
            }
            if let Some(until) = e.data["until"].as_str() {
                if best.as_deref().is_none_or(|b| until > b) {
                    best = Some(until.to_string());
                }
            }
        }
        Ok(best)
    }

    /// Each thread's subject, taken from the message that opened it.
    fn thread_topics(&self) -> Result<std::collections::HashMap<String, String>, Error> {
        let mut out = std::collections::HashMap::new();
        for m in self.messages()? {
            if let Some(t) = m.topic {
                out.insert(m.thread, t); // messages() is newest-first, so the opener lands last
            }
        }
        Ok(out)
    }

    /// The rooms `me` can see, most recently active first, each with its unread count — so an
    /// agent can find the right conversation instead of opening yet another one.
    pub fn rooms(&self, me: &[String], limit: usize) -> Result<Vec<Room>, Error> {
        let watermark = self.read_watermark(me)?;
        let is_me = |s: &str| me.iter().any(|m| m == s);
        let topics = self.thread_topics()?;

        let mut order: Vec<String> = Vec::new();
        let mut by_thread: std::collections::HashMap<String, Vec<Message>> =
            std::collections::HashMap::new();
        for m in self.messages()? {
            if !by_thread.contains_key(&m.thread) {
                order.push(m.thread.clone());
            }
            by_thread.entry(m.thread.clone()).or_default().push(m);
        }

        let mut rooms = Vec::new();
        for id in order {
            let Some(msgs) = by_thread.remove(&id) else {
                continue;
            };
            let mine = msgs
                .iter()
                .any(|m| m.to == "all" || is_me(&m.to) || is_me(&m.from));
            if !mine {
                continue;
            }
            let unread = msgs
                .iter()
                .filter(|m| !is_me(&m.from))
                .filter(|m| watermark.as_deref().is_none_or(|w| m.ts.as_str() > w))
                .count();
            let mut participants: Vec<String> = msgs
                .iter()
                .map(|m| m.from.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            participants.sort();
            let last = &msgs[0]; // newest-first
            rooms.push(Room {
                thread: id,
                topic: topics
                    .get(&last.thread)
                    .cloned()
                    .unwrap_or_else(|| short(&msgs[msgs.len() - 1].body, 40)),
                participants,
                last_ts: last.ts.clone(),
                messages: msgs.len(),
                unread,
                last_from: last.from.clone(),
                last_body: short(&last.body, 100),
            });
            if rooms.len() >= limit {
                break;
            }
        }
        Ok(rooms)
    }

    /// How many messages are waiting for `me`.
    pub fn unread_count(&self, me: &[String]) -> Result<usize, Error> {
        Ok(self.unread(me, usize::MAX)?.len())
    }

    /// Every conversation, most-recently-active first, each with its messages oldest-first — for
    /// showing the channel. Capped at `limit` threads.
    pub fn channel_threads(&self, limit: usize) -> Result<Vec<Vec<Message>>, Error> {
        let mut order: Vec<String> = Vec::new();
        let mut by_thread: std::collections::HashMap<String, Vec<Message>> =
            std::collections::HashMap::new();
        for m in self.messages()? {
            // messages() is newest-first, so first sighting of a thread = its latest activity.
            if !by_thread.contains_key(&m.thread) {
                order.push(m.thread.clone());
            }
            by_thread.entry(m.thread.clone()).or_default().push(m);
        }
        Ok(order
            .into_iter()
            .take(limit)
            .filter_map(|t| {
                by_thread.remove(&t).map(|mut ms| {
                    ms.reverse(); // oldest-first within the thread
                    ms
                })
            })
            .collect())
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
                topic: e
                    .data
                    .get("topic")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
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

#[cfg(test)]
mod room_tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::init(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn separate_subjects_stay_in_separate_rooms() {
        let (_d, store) = store();
        store
            .post_to_room(
                "claude",
                "all",
                None,
                "ask",
                "opaque tokens?",
                Some("auth-refactor"),
            )
            .unwrap();
        store
            .post_to_room(
                "claude",
                "all",
                None,
                "ask",
                "freeze billing",
                Some("billing-migration"),
            )
            .unwrap();

        let rooms = store.rooms(&["codex".into()], 10).unwrap();
        assert_eq!(
            rooms.len(),
            2,
            "two subjects must not collapse into one thread"
        );
        let topics: BTreeSet<&str> = rooms.iter().map(|r| r.topic.as_str()).collect();
        assert!(
            topics.contains("auth-refactor") && topics.contains("billing-migration"),
            "{topics:?}"
        );
    }

    #[test]
    fn many_agents_can_hold_one_conversation() {
        let (_d, store) = store();
        let room = store
            .post_to_room(
                "claude",
                "all",
                None,
                "ask",
                "who parses the JWT?",
                Some("auth-refactor"),
            )
            .unwrap();
        store
            .post_message("codex", "all", Some(&room), "reply", "my client does")
            .unwrap();
        store
            .post_message("cursor", "all", Some(&room), "reply", "the mobile app too")
            .unwrap();

        let rooms = store.rooms(&["claude".into()], 10).unwrap();
        let r = rooms.iter().find(|r| r.thread == room).unwrap();
        assert_eq!(r.messages, 3);
        assert_eq!(
            r.participants,
            vec!["claude".to_string(), "codex".into(), "cursor".into()],
            "everyone who spoke is in the room"
        );
    }

    #[test]
    fn unread_shows_every_missed_message_not_just_the_latest() {
        let (_d, store) = store();
        let room = store
            .post_to_room("claude", "codex", None, "ask", "first thing", Some("auth"))
            .unwrap();
        store
            .post_message("claude", "codex", Some(&room), "ask", "second thing")
            .unwrap();
        store
            .post_message("claude", "codex", Some(&room), "ask", "third thing")
            .unwrap();

        // Showing only the newest per thread would silently drop the first two.
        let unread = store.unread(&["codex".into()], 30).unwrap();
        assert_eq!(
            unread.len(),
            3,
            "codex must see everything it missed, not just the last message"
        );
        assert_eq!(
            unread[0].body, "first thing",
            "a conversation reads oldest-first"
        );
        assert_eq!(unread[2].body, "third thing");
    }

    #[test]
    fn reading_marks_read_and_new_messages_show_up_again() {
        let (_d, store) = store();
        let room = store
            .post_to_room("claude", "codex", None, "ask", "before", Some("auth"))
            .unwrap();
        assert_eq!(store.unread_count(&["codex".into()]).unwrap(), 1);

        store.mark_read("codex").unwrap();
        assert_eq!(
            store.unread_count(&["codex".into()]).unwrap(),
            0,
            "caught up"
        );

        store
            .post_message("claude", "codex", Some(&room), "ask", "after")
            .unwrap();
        let unread = store.unread(&["codex".into()], 10).unwrap();
        assert_eq!(unread.len(), 1, "only what arrived since");
        assert_eq!(unread[0].body, "after");
    }

    #[test]
    fn an_agent_is_never_notified_about_its_own_messages() {
        let (_d, store) = store();
        store
            .post_to_room("claude", "all", None, "ask", "anyone there?", Some("auth"))
            .unwrap();
        assert_eq!(
            store.unread_count(&["claude".into()]).unwrap(),
            0,
            "you do not have mail from yourself"
        );
        assert_eq!(store.unread_count(&["codex".into()]).unwrap(), 1);
    }
}
