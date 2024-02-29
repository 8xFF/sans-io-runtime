/// Task system for handling network and bus events.
///
/// Each task is a separate state machine that can receive network and bus events, and produce output events.
/// Tasks are grouped together into a task group, which is processed by a worker.
/// The worker is responsible for dispatching network and bus events to the appropriate task group, and for
/// processing the task groups.
///
use std::{net::SocketAddr, ops::Deref, time::Instant};

use crate::{backend::BackendIncoming, bus::BusEvent};

pub mod group;
pub mod group_state;

/// Represents an incoming network event.
pub enum NetIncoming<'a> {
    UdpListenResult {
        bind: SocketAddr,
        result: Result<SocketAddr, std::io::Error>,
    },
    UdpPacket {
        from: SocketAddr,
        to: SocketAddr,
        data: &'a [u8],
    },
}

impl<'a> NetIncoming<'a> {
    pub fn from_backend(event: BackendIncoming, buf: &'a [u8]) -> Self {
        match event {
            BackendIncoming::UdpListenResult { bind, result } => {
                Self::UdpListenResult { bind, result }
            }
            BackendIncoming::UdpPacket { from, to, len } => {
                let data = &buf[..len];
                Self::UdpPacket { from, to, data }
            }
        }
    }
}

pub enum Buffer<'a> {
    Ref(&'a [u8]),
    Vec(Vec<u8>),
}

impl<'a> Deref for Buffer<'a> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        match self {
            Buffer::Ref(data) => data,
            Buffer::Vec(data) => data,
        }
    }
}

/// Represents an outgoing network event.
pub enum NetOutgoing<'a> {
    UdpListen(SocketAddr),
    UdpPacket {
        from: SocketAddr,
        to: SocketAddr,
        data: Buffer<'a>,
    },
}

/// Represents an input event for a task.
pub enum TaskInput<'a, ChannelId, Event> {
    Net(NetIncoming<'a>),
    Bus(ChannelId, Event),
}

/// Represents an output event for a task.
pub enum TaskOutput<'a, ChannelId, Event> {
    Net(NetOutgoing<'a>),
    Bus(BusEvent<ChannelId, Event>),
    Destroy,
}

/// Represents a task.
pub trait Task<ChannelId, Event> {
    /// The type identifier for the task.
    const TYPE: u16;

    /// Called each time the task is ticked. Default is 1ms.
    fn on_tick<'a>(&mut self, now: Instant) -> Option<TaskOutput<'a, ChannelId, Event>>;

    /// Called when an input event is received for the task.
    fn on_input<'a>(
        &mut self,
        now: Instant,
        input: TaskInput<'a, ChannelId, Event>,
    ) -> Option<TaskOutput<'a, ChannelId, Event>>;

    /// Retrieves the next output event from the task.
    fn pop_output<'a>(&mut self, now: Instant) -> Option<TaskOutput<'a, ChannelId, Event>>;
}
