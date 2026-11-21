use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use rmcp::{
    ErrorData as McpError,
    model::{DetailedTask, Task, TaskPayload, TaskStatus},
    task_manager::{DEFAULT_POLL_INTERVAL_MS, current_timestamp},
};
use tokio_util::sync::CancellationToken;

pub(super) type ConnectionTasks = Arc<Mutex<TaskStore>>;

// rmcp's TaskManager can expire live work and has no settled-record count limit.
const RETENTION: Duration = Duration::from_secs(3600);
const TERMINAL_LIMIT: usize = 20;

struct ActiveTask {
    task: Task,
    created: Instant,
    cancellation: CancellationToken,
}

struct SettledTask {
    settled_at: Instant,
    detail: Arc<DetailedTask>,
}

#[derive(Default)]
pub(super) struct TaskStore {
    active: HashMap<String, ActiveTask>,
    terminal: VecDeque<SettledTask>,
}

impl TaskStore {
    pub(super) fn register(
        &mut self,
        id: String,
        cancellation: CancellationToken,
        now: Instant,
    ) -> Task {
        self.prune(now);
        let timestamp = current_timestamp();
        let task = Task::new(
            id.clone(),
            TaskStatus::Working,
            timestamp.clone(),
            timestamp,
        )
        .with_poll_interval_ms(DEFAULT_POLL_INTERVAL_MS);
        self.active.insert(
            id,
            ActiveTask {
                task: task.clone(),
                created: now,
                cancellation,
            },
        );
        task
    }

    pub(super) fn finish(&mut self, id: &str, payload: TaskPayload, now: Instant) {
        self.prune(now);
        let Some(ActiveTask {
            mut task, created, ..
        }) = self.active.remove(id)
        else {
            return;
        };
        let lifetime = now.duration_since(created).saturating_add(RETENTION);
        task.ttl_ms = Some(u64::try_from(lifetime.as_millis()).unwrap_or(u64::MAX));
        task.last_updated_at = current_timestamp();
        if self.terminal.len() == TERMINAL_LIMIT {
            self.terminal.pop_front();
        }
        self.terminal.push_back(SettledTask {
            settled_at: now,
            detail: Arc::new(DetailedTask::new(task, payload)),
        });
    }

    fn prune(&mut self, now: Instant) {
        while self
            .terminal
            .front()
            .is_some_and(|record| now.duration_since(record.settled_at) >= RETENTION)
        {
            self.terminal.pop_front();
        }
    }

    fn terminal(&self, id: &str) -> Option<&Arc<DetailedTask>> {
        self.terminal
            .iter()
            .map(|record| &record.detail)
            .find(|task| task.task.task_id == id)
    }

    pub(super) fn get(&mut self, id: &str, now: Instant) -> Result<Arc<DetailedTask>, McpError> {
        self.prune(now);
        if let Some(active) = self.active.get(id) {
            return Ok(Arc::new(DetailedTask::new(
                active.task.clone(),
                TaskPayload::Working,
            )));
        }
        self.terminal(id).cloned().ok_or_else(unknown_task)
    }

    pub(super) fn acknowledge_update(&mut self, id: &str, now: Instant) -> Result<(), McpError> {
        self.active_token(id, now).map(|_| ())
    }

    pub(super) fn request_cancel(&mut self, id: &str, now: Instant) -> Result<(), McpError> {
        if let Some(token) = self.active_token(id, now)? {
            token.cancel();
        }
        Ok(())
    }

    fn active_token(
        &mut self,
        id: &str,
        now: Instant,
    ) -> Result<Option<&CancellationToken>, McpError> {
        self.prune(now);
        if let Some(active) = self.active.get(id) {
            Ok(Some(&active.cancellation))
        } else {
            self.terminal(id).map(|_| None).ok_or_else(unknown_task)
        }
    }
}

fn unknown_task() -> McpError {
    McpError::invalid_params("Unknown task ID", None)
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    impl TaskStore {
        pub(in crate::mcp) fn is_empty(&self) -> bool {
            self.active.is_empty() && self.terminal.is_empty()
        }
    }

    fn register(store: &mut TaskStore, now: Instant) -> Task {
        store.register(Uuid::new_v4().to_string(), CancellationToken::new(), now)
    }

    #[test]
    fn ids_are_registered_only_in_their_own_store() {
        let mut store = TaskStore::default();
        let mut other = TaskStore::default();
        let now = Instant::now();
        let task = register(&mut store, now);
        assert_ne!(task.task_id, register(&mut other, now).task_id);
        assert_eq!(store.get(&task.task_id, now).unwrap().task, task);
        assert!(other.get(&task.task_id, now).is_err());
    }

    #[test]
    fn duplicate_or_unknown_settlement_creates_no_record() {
        let mut store = TaskStore::default();
        let now = Instant::now();
        let id = register(&mut store, now).task_id;
        store.finish("absent", TaskPayload::Cancelled, now);
        assert!(store.terminal.is_empty());
        store.finish(&id, TaskPayload::Cancelled, now);
        store.finish(&id, TaskPayload::Working, now);
        assert_eq!(store.terminal.len(), 1);
        assert_eq!(store.get(&id, now).unwrap().status(), TaskStatus::Cancelled);
    }

    #[test]
    fn terminal_lifetime_is_creation_relative_but_expiry_is_relative_to_settlement() {
        let mut store = TaskStore::default();
        let created = Instant::now();
        let first = register(&mut store, created).task_id;
        let second = register(&mut store, created).task_id;
        let running = register(&mut store, created).task_id;
        let settled = created + RETENTION * 2;
        store.finish(&first, TaskPayload::Cancelled, settled);
        store.finish(&second, TaskPayload::Cancelled, settled);
        let last_access = settled + RETENTION - Duration::from_millis(1);
        assert_eq!(
            store.get(&first, last_access).unwrap().task.ttl_ms,
            Some(10_800_000)
        );
        store.request_cancel(&second, last_access).unwrap();
        let boundary = settled + RETENTION;
        assert!(store.get(&first, boundary).is_err());
        assert!(store.acknowledge_update(&second, boundary).is_err());
        assert_eq!(store.get(&running, boundary).unwrap().task.ttl_ms, None);

        store.finish(&running, TaskPayload::Cancelled, boundary);
        let next = register(&mut store, boundary).task_id;
        store.finish(&next, TaskPayload::Cancelled, boundary + RETENTION);
        assert_eq!(
            store.terminal.len(),
            1,
            "settlement must prune without a poll"
        );
    }

    #[test]
    fn only_settlement_order_counts_towards_capacity() {
        let mut store = TaskStore::default();
        let now = Instant::now();
        let running = register(&mut store, now).task_id;
        let ids: Vec<_> = (0..21).map(|_| register(&mut store, now).task_id).collect();
        for (n, id) in ids.iter().rev().enumerate() {
            let payload = match n % 3 {
                0 => TaskPayload::Failed {
                    error: Default::default(),
                },
                1 => TaskPayload::Completed {
                    result: Default::default(),
                },
                _ => TaskPayload::Cancelled,
            };
            store.finish(id, payload, now);
        }
        assert!(
            store.get(&ids[20], now).is_err(),
            "first settled, even if never polled"
        );
        assert_eq!(store.terminal.len(), 20);
        let expected = [
            TaskStatus::Completed,
            TaskStatus::Cancelled,
            TaskStatus::Failed,
        ]
        .into_iter()
        .cycle();
        for (id, status) in ids[..20].iter().rev().zip(expected) {
            assert_eq!(store.get(id, now).unwrap().status(), status);
        }
        assert_eq!(
            store.get(&running, now).unwrap().status(),
            TaskStatus::Working
        );
    }
}
