use std::{net::SocketAddr, sync::Arc, time::Duration};

use crate::{owner::Owner, task::NetOutgoing};

#[cfg(feature = "mio-backend")]
mod mio;

#[cfg(feature = "poll-backend")]
mod poll;

#[cfg(feature = "mio-backend")]
pub use mio::MioBackend;

#[cfg(feature = "poll-backend")]
pub use poll::PollBackend;

/// Represents an incoming network event.
#[derive(Debug)]
pub enum BackendIncoming {
    UdpListenResult {
        bind: SocketAddr,
        result: Result<(SocketAddr, usize), std::io::Error>,
    },
    UdpPacket {
        slot: usize,
        from: SocketAddr,
        len: usize,
    },
    Awake,
}

pub trait Awaker: Send + Sync {
    fn awake(&self);
}

pub trait Backend: Default + BackendOwner {
    fn create_awaker(&self) -> Arc<dyn Awaker>;
    fn poll_incoming(&mut self, timeout: Duration);
    fn pop_incoming(&mut self, buf: &mut [u8]) -> Option<(BackendIncoming, Owner)>;
    fn finish_outgoing_cycle(&mut self);
    fn finish_incoming_cycle(&mut self);
}

pub trait BackendOwner {
    fn on_action(&mut self, owner: Owner, action: NetOutgoing);
    fn remove_owner(&mut self, owner: Owner);
}
