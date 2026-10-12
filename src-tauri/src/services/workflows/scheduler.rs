// SOT: workflow-scheduler, schedule-arming, due-fires, schedule-status-list, schedules-active

use super::schedule;
use crate::error::AppResult;
use crate::model::workflow::{NodeConfig, ScheduleConfig};
use crate::model::{DocumentBody, DocumentKind, WorkflowScheduleStatus};
use crate::services::connection;
use crate::state::AppState;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

// ============================================================================
// WORKFLOW SCHEDULER
//
// WHAT:  Knows when each Schedule node fires next, and says which are due.
// WHY:   Schedules must fire with the workflow tab closed, so the clock lives in
//        the core, not the window. Only workflows with "Schedules active" on are
//        armed; a paused workflow keeps its nodes but never fires.
// HOW:   `poll` (called about once a second by the command layer) re-reads the
//        saved workflows when they changed, and returns the fires that are due.
//        A node whose settings did not change keeps its timer, so saving an
//        unrelated edit does not postpone an interval schedule. A fire is never
//        replayed: after the machine sleeps through one, the next fire is
//        computed from now.
// WHERE: src-tauri/src/commands/workflows.rs (the tick), src-tauri/src/services/documents.rs (marks it dirty)
// ============================================================================

/// How often the saved workflows are re-read even if nothing said they changed
/// (a duplicate or an import writes the store without telling the scheduler).
const RESCAN_EVERY_SECONDS: i64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Fire {
    pub workflow_id: String,
    pub node_id: String,
}

struct Armed {
    config: ScheduleConfig,
    signature: String,
    next: DateTime<Utc>,
}

#[derive(Default)]
struct Book {
    armed: HashMap<(String, String), Armed>,
    last_scan: Option<DateTime<Utc>>,
}

#[derive(Default)]
pub struct Scheduler {
    book: Mutex<Book>,
    dirty: AtomicBool,
}

static GLOBAL: OnceLock<Scheduler> = OnceLock::new();

/// The process-wide scheduler.
pub fn scheduler() -> &'static Scheduler {
    GLOBAL.get_or_init(|| Scheduler { book: Mutex::new(Book::default()), dirty: AtomicBool::new(true) })
}

/// A workflow was saved or deleted: re-read the schedules on the next tick.
pub fn documents_changed() {
    scheduler().mark_dirty();
}

impl Scheduler {
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::SeqCst);
    }

    // WHAT:  Fires that are due now, after re-reading the saved workflows if needed.
    pub fn poll(&self, state: &AppState, now: DateTime<Utc>) -> Vec<Fire> {
        let stale = self.book.lock().map(|b| b.last_scan.is_none_or(|at| (now - at).num_seconds() >= RESCAN_EVERY_SECONDS)).unwrap_or(true);
        if self.dirty.swap(false, Ordering::SeqCst) || stale {
            match wanted(state) {
                Ok(wanted) => self.apply(wanted, now),
                Err(err) => log::warn!("could not read workflow schedules: {err}"),
            }
        }
        self.take_due(now)
    }

    // WHAT:  Arms what should be armed and forgets the rest.
    pub fn apply(&self, wanted: HashMap<(String, String), ScheduleConfig>, now: DateTime<Utc>) {
        let Ok(mut book) = self.book.lock() else { return };
        book.last_scan = Some(now);
        book.armed.retain(|key, _| wanted.contains_key(key));
        for (key, config) in wanted {
            let signature = serde_json::to_string(&config).unwrap_or_default();
            if book.armed.get(&key).is_some_and(|a| a.signature == signature) {
                continue;
            }
            match schedule::next_fire(&config, now) {
                Some(next) => {
                    book.armed.insert(key, Armed { config, signature, next });
                }
                None => {
                    book.armed.remove(&key);
                }
            }
        }
    }

    // WHAT:  The armed schedules whose time has come, each re-armed for its next time.
    pub fn take_due(&self, now: DateTime<Utc>) -> Vec<Fire> {
        let Ok(mut book) = self.book.lock() else { return Vec::new() };
        let mut due = Vec::new();
        let mut gone = Vec::new();
        for (key, armed) in &mut book.armed {
            if armed.next > now {
                continue;
            }
            due.push(Fire { workflow_id: key.0.clone(), node_id: key.1.clone() });
            match schedule::next_fire(&armed.config, now) {
                Some(next) => armed.next = next,
                None => gone.push(key.clone()),
            }
        }
        for key in gone {
            book.armed.remove(&key);
        }
        due.sort_by(|a, b| (&a.workflow_id, &a.node_id).cmp(&(&b.workflow_id, &b.node_id)));
        due
    }

    pub fn next_fire_of(&self, workflow_id: &str, node_id: &str) -> Option<DateTime<Utc>> {
        let book = self.book.lock().ok()?;
        book.armed.get(&(workflow_id.to_string(), node_id.to_string())).map(|a| a.next)
    }
}

/// Every schedule that should be armed: Schedule nodes of workflows that have "Schedules active" on.
fn wanted(state: &AppState) -> AppResult<HashMap<(String, String), ScheduleConfig>> {
    let docs = state.with_store(|store| store.list_documents(DocumentKind::Workflow))?;
    let mut wanted = HashMap::new();
    for doc in docs {
        let DocumentBody::Workflow(body) = doc.body else { continue };
        if !body.schedules_active {
            continue;
        }
        for node in body.nodes {
            if let NodeConfig::Schedule(config) = node.config {
                if schedule::problem(&config).is_none() {
                    wanted.insert((doc.id.clone(), node.id), config);
                }
            }
        }
    }
    Ok(wanted)
}

// WHAT:  Every Schedule node in every workflow, active or paused, for Settings.
pub fn statuses(state: &AppState, now: DateTime<Utc>) -> AppResult<Vec<WorkflowScheduleStatus>> {
    let docs = state.with_store(|store| store.list_documents(DocumentKind::Workflow))?;
    let connections: HashMap<String, String> = connection::list(state)?.into_iter().map(|c| (c.id, c.name)).collect();
    let scheduler = scheduler();
    let mut out = Vec::new();
    for doc in docs {
        let DocumentBody::Workflow(body) = doc.body else { continue };
        for node in body.nodes {
            let NodeConfig::Schedule(config) = node.config else { continue };
            let next = if body.schedules_active && schedule::problem(&config).is_none() {
                scheduler.next_fire_of(&doc.id, &node.id).or_else(|| schedule::next_fire(&config, now))
            } else {
                None
            };
            out.push(WorkflowScheduleStatus {
                workflow_id: doc.id.clone(),
                workflow_name: doc.name.clone(),
                connection_id: doc.connection_id.clone(),
                connection_name: doc.connection_id.as_ref().and_then(|id| connections.get(id).cloned()),
                node_id: node.id,
                mode: config.mode,
                summary: schedule::problem(&config).unwrap_or_else(|| schedule::summarize(&config)),
                schedules_active: body.schedules_active,
                next_fire_at: next.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            });
        }
    }
    out.sort_by(|a, b| (a.workflow_name.to_lowercase(), &a.node_id).cmp(&(b.workflow_name.to_lowercase(), &b.node_id)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::workflow::{IntervalUnit, ScheduleMode};

    fn at(rfc3339: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(rfc3339).unwrap().with_timezone(&Utc)
    }

    fn every(minutes: u32) -> ScheduleConfig {
        ScheduleConfig { mode: ScheduleMode::Interval, interval_amount: minutes, interval_unit: IntervalUnit::Minutes, ..ScheduleConfig::default() }
    }

    fn key(w: &str, n: &str) -> (String, String) {
        (w.to_string(), n.to_string())
    }

    #[test]
    fn due_fires_are_returned_once_and_rearmed() {
        let s = Scheduler::default();
        let t0 = at("2026-01-01T00:00:00Z");
        s.apply(HashMap::from([(key("w", "n"), every(5))]), t0);
        assert!(s.take_due(at("2026-01-01T00:04:59Z")).is_empty());
        let fired = s.take_due(at("2026-01-01T00:05:00Z"));
        assert_eq!(fired, vec![Fire { workflow_id: "w".into(), node_id: "n".into() }]);
        assert!(s.take_due(at("2026-01-01T00:05:01Z")).is_empty(), "a fire is not repeated");
        assert_eq!(s.next_fire_of("w", "n"), Some(at("2026-01-01T00:10:00Z")));
    }

    #[test]
    fn a_missed_fire_is_not_replayed() {
        let s = Scheduler::default();
        s.apply(HashMap::from([(key("w", "n"), every(1))]), at("2026-01-01T00:00:00Z"));
        // The machine slept for an hour: one fire, then the next minute from now.
        assert_eq!(s.take_due(at("2026-01-01T01:00:00Z")).len(), 1);
        assert_eq!(s.next_fire_of("w", "n"), Some(at("2026-01-01T01:01:00Z")));
    }

    #[test]
    fn unchanged_schedules_keep_their_timer_and_removed_ones_go() {
        let s = Scheduler::default();
        s.apply(HashMap::from([(key("w", "n"), every(5)), (key("w", "gone"), every(5))]), at("2026-01-01T00:00:00Z"));
        // Re-applied a minute later with the same settings: the 00:05 fire is still 00:05.
        s.apply(HashMap::from([(key("w", "n"), every(5))]), at("2026-01-01T00:01:00Z"));
        assert_eq!(s.next_fire_of("w", "n"), Some(at("2026-01-01T00:05:00Z")));
        assert_eq!(s.next_fire_of("w", "gone"), None);
        // Changing the settings re-arms from now.
        s.apply(HashMap::from([(key("w", "n"), every(10))]), at("2026-01-01T00:02:00Z"));
        assert_eq!(s.next_fire_of("w", "n"), Some(at("2026-01-01T00:12:00Z")));
    }

    #[test]
    fn a_schedule_that_cannot_fire_is_not_armed() {
        let s = Scheduler::default();
        s.apply(HashMap::from([(key("w", "n"), every(0))]), at("2026-01-01T00:00:00Z"));
        assert_eq!(s.next_fire_of("w", "n"), None);
        assert!(s.take_due(at("2030-01-01T00:00:00Z")).is_empty());
    }
}
