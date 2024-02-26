use std::time::Duration;

use crate::{
    owner::Owner,
    task::{NetIncoming, NetOutgoing},
};

#[cfg(feature = "mio-backend")]
mod mio;

#[cfg(feature = "mio-backend")]
pub use mio::MioBackend;

pub trait Backend: Default + BackendOwner {
    fn pop_incoming(&mut self, timeout: Duration) -> Option<(NetIncoming, Owner)>;
    fn finish_outgoing_cycle(&mut self);
    fn finish_incoming_cycle(&mut self);
}

pub trait BackendOwner {
    fn on_action(&mut self, action: NetOutgoing, owner: Owner);
    fn remove_owner(&mut self, owner: Owner);
    fn swap_owner(&mut self, from: Owner, to: Owner);
}
