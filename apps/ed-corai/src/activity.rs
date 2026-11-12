//! What the window knows: connected AI apps, running tool calls and recent results.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant, SystemTime};

use corai_activity::{Event, Message};

/// How many finished calls the window keeps.
pub const HISTORY: usize = 50;

/// An AI app whose ED Corai server is connected.
#[derive(Debug, Clone, PartialEq)]
pub struct Client {
    pub name: String,
    pub pid: u32,
}

/// A tool call in progress.
#[derive(Debug, Clone)]
pub struct Job {
    pub client: String,
    pub tool: String,
    pub detail: String,
    pub started: Instant,
}

/// A finished tool call.
#[derive(Debug, Clone)]
pub struct Done {
    pub client: String,
    pub tool: String,
    pub detail: String,
    pub ok: bool,
    pub took: Duration,
    pub link: Option<String>,
    pub at: SystemTime,
}

#[derive(Debug, Default)]
pub struct Activity {
    pub clients: BTreeMap<u64, Client>,
    /// Keyed by connection and call id.
    pub jobs: BTreeMap<(u64, u64), Job>,
    /// Newest first.
    pub history: VecDeque<Done>,
}

impl Activity {
    pub fn apply(&mut self, event: Event, now: Instant) {
        match event {
            Event::Connected { conn, client, pid } => {
                self.clients.insert(conn, Client { name: client, pid });
            }
            Event::Disconnected { conn } => {
                self.clients.remove(&conn);
                self.jobs.retain(|(c, _), _| *c != conn);
            }
            Event::Message { conn, message } => self.message(conn, message, now),
            _ => {}
        }
    }

    fn client_name(&self, conn: u64) -> String {
        self.clients
            .get(&conn)
            .map_or_else(|| "an AI app".into(), |c| c.name.clone())
    }

    fn message(&mut self, conn: u64, message: Message, now: Instant) {
        match message {
            Message::Client { name } => {
                if let Some(c) = self.clients.get_mut(&conn) {
                    c.name = name;
                }
            }
            Message::Started { id, tool, detail } => {
                let client = self.client_name(conn);
                self.jobs.insert(
                    (conn, id),
                    Job {
                        client,
                        tool,
                        detail,
                        started: now,
                    },
                );
            }
            Message::Finished {
                id,
                ok,
                millis,
                detail,
                link,
            } => {
                let job = self.jobs.remove(&(conn, id));
                let client = self.client_name(conn);
                self.history.push_front(Done {
                    client,
                    tool: job.map(|j| j.tool).unwrap_or_default(),
                    detail,
                    ok,
                    took: Duration::from_millis(millis),
                    link,
                    at: SystemTime::now(),
                });
                self.history.truncate(HISTORY);
            }
            _ => {}
        }
    }

    pub fn busy(&self) -> bool {
        !self.jobs.is_empty()
    }

    /// Distinct names of the connected AI apps.
    pub fn client_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.clients.values().map(|c| c.name.clone()).collect();
        names.sort();
        names.dedup();
        names
    }
}

/// A tool name as people read it: `optimize_build` → "Optimise build".
pub fn tool_label(tool: &str) -> String {
    let label = match tool {
        "optimize_build" => "optimise build",
        "compare_ships" => "compare ships",
        "evaluate_build" => "evaluate build",
        "export_build" => "export build",
        "ship_details" => "ship details",
        "module_info" => "module info",
        other => return capitalise(&other.replace('_', " ")),
    };
    capitalise(label)
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// "0.4 s", "12 s", "2 min 5 s".
pub fn duration_label(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 10.0 {
        format!("{s:.1} s")
    } else if s < 60.0 {
        format!("{s:.0} s")
    } else {
        format!("{} min {} s", d.as_secs() / 60, d.as_secs() % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(conn: u64, id: u64) -> Event {
        Event::Message {
            conn,
            message: Message::Started {
                id,
                tool: "optimize_build".into(),
                detail: "python".into(),
            },
        }
    }

    #[test]
    fn jobs_run_between_start_and_finish() {
        let mut a = Activity::default();
        let now = Instant::now();
        a.apply(
            Event::Connected {
                conn: 1,
                client: "Claude Code".into(),
                pid: 7,
            },
            now,
        );
        a.apply(started(1, 1), now);
        assert!(a.busy());
        assert_eq!(a.jobs[&(1, 1)].client, "Claude Code");
        a.apply(
            Event::Message {
                conn: 1,
                message: Message::Finished {
                    id: 1,
                    ok: true,
                    millis: 1500,
                    detail: "Python: jumpLaden 31".into(),
                    link: Some("u".into()),
                },
            },
            now,
        );
        assert!(!a.busy());
        let d = &a.history[0];
        assert_eq!(
            (d.tool.as_str(), d.ok, d.took),
            ("optimize_build", true, Duration::from_millis(1500))
        );
    }

    #[test]
    fn a_closed_connection_takes_its_jobs_with_it() {
        let mut a = Activity::default();
        let now = Instant::now();
        a.apply(
            Event::Connected {
                conn: 1,
                client: "A".into(),
                pid: 1,
            },
            now,
        );
        a.apply(
            Event::Connected {
                conn: 2,
                client: "B".into(),
                pid: 2,
            },
            now,
        );
        a.apply(started(1, 1), now);
        a.apply(started(2, 1), now);
        a.apply(Event::Disconnected { conn: 1 }, now);
        assert_eq!(a.jobs.len(), 1);
        assert_eq!(a.client_names(), vec!["B".to_string()]);
    }

    #[test]
    fn the_client_can_rename_itself_and_history_is_bounded() {
        let mut a = Activity::default();
        let now = Instant::now();
        a.apply(
            Event::Connected {
                conn: 1,
                client: "an AI app".into(),
                pid: 1,
            },
            now,
        );
        a.apply(
            Event::Message {
                conn: 1,
                message: Message::Client {
                    name: "Claude Desktop".into(),
                },
            },
            now,
        );
        assert_eq!(a.client_names(), vec!["Claude Desktop".to_string()]);
        for id in 0..(HISTORY as u64 + 5) {
            a.apply(started(1, id), now);
            a.apply(
                Event::Message {
                    conn: 1,
                    message: Message::Finished {
                        id,
                        ok: true,
                        millis: 1,
                        detail: String::new(),
                        link: None,
                    },
                },
                now,
            );
        }
        assert_eq!(a.history.len(), HISTORY);
    }

    #[test]
    fn labels_read_naturally() {
        assert_eq!(tool_label("optimize_build"), "Optimise build");
        assert_eq!(tool_label("list_ships"), "List ships");
        assert_eq!(duration_label(Duration::from_millis(400)), "0.4 s");
        assert_eq!(duration_label(Duration::from_secs(125)), "2 min 5 s");
    }
}
