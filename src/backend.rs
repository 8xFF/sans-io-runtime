use std::{net::SocketAddr, time::Duration};

use crate::{owner::Owner, task::NetOutgoing};

#[cfg(feature = "mio-backend")]
mod mio;

#[cfg(feature = "mio-backend")]
pub use mio::MioBackend;

/// Represents an incoming network event.
pub enum BackendIncoming {
    UdpListenResult {
        bind: SocketAddr,
        result: Result<SocketAddr, std::io::Error>,
    },
    UdpPacket {
        from: SocketAddr,
        to: SocketAddr,
        len: usize,
    },
}

pub trait Backend: Default + BackendOwner {
    fn pop_incoming(
        &mut self,
        timeout: Duration,
        buf: &mut [u8],
    ) -> Option<(BackendIncoming, Owner)>;
    fn finish_outgoing_cycle(&mut self);
    fn finish_incoming_cycle(&mut self);
}

pub trait BackendOwner {
    fn on_action<'a>(&mut self, owner: Owner, action: NetOutgoing<'a>);
    fn remove_owner(&mut self, owner: Owner);
}
