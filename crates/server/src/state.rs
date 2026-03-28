use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

use bytes::Bytes;
use dashmap::DashMap;
use tokio::sync::mpsc;

use common::proto::{ControlMsg, TunnelSpec};

/// One half of a bidirectional stream: bytes going toward the local service.
pub type StreamTx = mpsc::Sender<Bytes>;

#[derive(Debug)]
pub struct ServerState {
    /// Send a ControlMsg to the connected client over the control WebSocket.
    pub control_tx: mpsc::Sender<ControlMsg>,

    /// Active streams keyed by stream_id.
    pub streams: DashMap<u32, StreamTx>,

    /// Tunnels the client registered.
    #[allow(dead_code)]
    pub tunnels: Vec<TunnelSpec>,

    next_stream_id: AtomicU32,
}

impl ServerState {
    pub fn new(control_tx: mpsc::Sender<ControlMsg>, tunnels: Vec<TunnelSpec>) -> Arc<Self> {
        Arc::new(Self {
            control_tx,
            streams: DashMap::new(),
            tunnels,
            next_stream_id: AtomicU32::new(1),
        })
    }

    pub fn next_stream_id(&self) -> u32 {
        self.next_stream_id.fetch_add(1, Ordering::Relaxed)
    }
}
