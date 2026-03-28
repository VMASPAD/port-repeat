use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use tokio::time::Duration;
use tracing::debug;

use crate::state::ServerState;

struct HttpTunnelEntry {
    state: Arc<ServerState>,
    tunnel_id: u8,
    last_active_ms: AtomicU64,
}

pub struct HttpRegistry {
    entries: DashMap<String, Arc<HttpTunnelEntry>>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl HttpRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: DashMap::new(),
        })
    }

    /// Register a UUID endpoint pointing to the given session state.
    pub fn register_with_uuid(&self, state: Arc<ServerState>, tunnel_id: u8, uuid: String) {
        let entry = Arc::new(HttpTunnelEntry {
            state,
            tunnel_id,
            last_active_ms: AtomicU64::new(now_ms()),
        });
        self.entries.insert(uuid, entry);
    }

    /// Remove all UUID entries belonging to the session identified by its allocation address.
    pub fn deregister_session(&self, state_addr: usize) {
        self.entries
            .retain(|_, entry| Arc::as_ptr(&entry.state) as usize != state_addr);
    }

    /// Look up a UUID and return its session state + tunnel_id if found.
    pub fn lookup(&self, uuid: &str) -> Option<(Arc<ServerState>, u8)> {
        self.entries
            .get(uuid)
            .map(|entry| (Arc::clone(&entry.state), entry.tunnel_id))
    }

    /// Update the last-active timestamp for a UUID (called on each new HTTP connection).
    pub fn touch(&self, uuid: &str) {
        if let Some(entry) = self.entries.get(uuid) {
            entry.last_active_ms.store(now_ms(), Ordering::Relaxed);
        }
    }

    /// Background task: periodically removes entries idle longer than `timeout_secs`.
    pub async fn run_idle_reaper(self: Arc<Self>, timeout_secs: u64) {
        let period = Duration::from_secs((timeout_secs / 2).max(10));
        loop {
            tokio::time::sleep(period).await;
            let now = now_ms();
            let threshold_ms = timeout_secs * 1000;
            self.entries.retain(|uuid, entry| {
                let age = now.saturating_sub(entry.last_active_ms.load(Ordering::Relaxed));
                if age >= threshold_ms {
                    debug!("idle timeout expired for UUID {uuid}");
                    false
                } else {
                    true
                }
            });
        }
    }
}
