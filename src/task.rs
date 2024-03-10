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
#[derive(Debug)]
pub enum NetIncoming<'a> {
    UdpListenResult {
        bind: SocketAddr,
        result: Result<(SocketAddr, usize), std::io::Error>,
    },
    UdpPacket {
        slot: usize,
        from: SocketAddr,
        data: &'a [u8],
    },
}

impl<'a> NetIncoming<'a> {
    pub fn from_backend(event: BackendIncoming, buf: &'a [u8]) -> Self {
        match event {
            BackendIncoming::UdpListenResult { bind, result } => {
                Self::UdpListenResult { bind, result }
            }
            BackendIncoming::UdpPacket { from, slot, len } => {
                let data = &buf[..len];
                Self::UdpPacket { from, slot, data }
            }
            BackendIncoming::Awake => {
                panic!("Unexpected awake event");
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
    UdpListen {
        addr: SocketAddr,
        reuse: bool,
    },
    UdpUnlisten {
        slot: usize,
    },
    UdpPacket {
        slot: usize,
        to: SocketAddr,
        data: Buffer<'a>,
    },
}

/// Represents an input event for a task.
#[derive(Debug)]
pub enum TaskInput<'a, ChannelId, Event> {
    Net(NetIncoming<'a>),
    Bus(ChannelId, Event),
}

impl<'a, ChannelId, Event> TaskInput<'a, ChannelId, Event> {
    pub fn convert_into<IChannelId, IEvent>(self) -> Option<TaskInput<'a, IChannelId, IEvent>>
    where
        ChannelId: TryInto<IChannelId>,
        Event: TryInto<IEvent>,
    {
        match self {
            TaskInput::Net(net) => Some(TaskInput::Net(net)),
            TaskInput::Bus(channel, event) => Some(TaskInput::Bus(
                channel.try_into().ok()?,
                event.try_into().ok()?,
            )),
        }
    }
}

/// Represents an output event for a task.
pub enum TaskOutput<'a, ChannelIn, ChannelOut, Event> {
    Net(NetOutgoing<'a>),
    Bus(BusEvent<ChannelIn, ChannelOut, Event>),
    Destroy,
}

impl<'a, ChannelIn, ChannelOut, Event> TaskOutput<'a, ChannelIn, ChannelOut, Event> {
    pub fn convert_into<
        NChannelIn: From<ChannelIn>,
        NChannelOut: From<ChannelOut>,
        NEvent: From<Event>,
    >(
        self,
    ) -> TaskOutput<'a, NChannelIn, NChannelOut, NEvent> {
        match self {
            TaskOutput::Net(net) => TaskOutput::Net(match net {
                NetOutgoing::UdpListen { addr, reuse } => NetOutgoing::UdpListen { addr, reuse },
                NetOutgoing::UdpUnlisten { slot } => NetOutgoing::UdpUnlisten { slot },
                NetOutgoing::UdpPacket { slot, to, data } => {
                    NetOutgoing::UdpPacket { slot, to, data }
                }
            }),
            TaskOutput::Bus(event) => TaskOutput::Bus(event.convert_into()),
            TaskOutput::Destroy => TaskOutput::Destroy,
        }
    }
}

/// Represents a task.
pub trait Task<ChannelIn, ChannelOut, EventIn, EventOut> {
    /// The type identifier for the task.
    const TYPE: u16;

    /// Called each time the task is ticked. Default is 1ms.
    fn on_tick<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskOutput<'a, ChannelIn, ChannelOut, EventOut>>;

    /// Called when an input event is received for the task.
    fn on_event<'a>(
        &mut self,
        now: Instant,
        input: TaskInput<'a, ChannelIn, EventIn>,
    ) -> Option<TaskOutput<'a, ChannelIn, ChannelOut, EventOut>>;

    /// Retrieves the next output event from the task.
    fn pop_output<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskOutput<'a, ChannelIn, ChannelOut, EventOut>>;

    /// Gracefully shuts down the task.
    fn shutdown<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskOutput<'a, ChannelIn, ChannelOut, EventOut>>;
}
