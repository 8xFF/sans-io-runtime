#[cfg(feature = "tun-tap")]
use crate::backend::tun::TunFd;
/// Task system for handling network and bus events.
///
/// Each task is a separate state machine that can receive network and bus events, and produce output events.
/// Tasks are grouped together into a task group, which is processed by a worker.
/// The worker is responsible for dispatching network and bus events to the appropriate task group, and for
/// processing the task groups.
///
#[cfg(feature = "udp")]
use std::net::SocketAddr;

use std::{ops::Deref, time::Instant};

use crate::{backend::BackendIncomingEvent, bus::BusEvent};

pub mod group;
pub mod switcher;

/// Represents an incoming network event.
#[derive(Debug)]
pub enum NetIncoming<'a> {
    #[cfg(feature = "udp")]
    UdpListenResult {
        bind: SocketAddr,
        result: Result<(SocketAddr, usize), std::io::Error>,
    },
    #[cfg(feature = "udp")]
    UdpPacket {
        slot: usize,
        from: SocketAddr,
        data: &'a mut [u8],
    },
    #[cfg(feature = "tun-tap")]
    TunBindResult {
        result: Result<usize, std::io::Error>,
    },
    #[cfg(feature = "tun-tap")]
    TunPacket { slot: usize, data: &'a mut [u8] },
}

impl<'a> NetIncoming<'a> {
    pub fn from_backend(event: BackendIncomingEvent, buf: &'a mut [u8]) -> Self {
        match event {
            #[cfg(feature = "udp")]
            BackendIncomingEvent::UdpListenResult { bind, result } => {
                Self::UdpListenResult { bind, result }
            }
            #[cfg(feature = "udp")]
            BackendIncomingEvent::UdpPacket { from, slot, len } => {
                let data = &mut buf[..len];
                Self::UdpPacket { from, slot, data }
            }
            #[cfg(feature = "tun-tap")]
            BackendIncomingEvent::TunBindResult { result } => Self::TunBindResult { result },
            #[cfg(feature = "tun-tap")]
            BackendIncomingEvent::TunPacket { slot, len } => {
                let data = &mut buf[..len];
                Self::TunPacket { slot, data }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum Buffer<'a> {
    Ref(&'a [u8]),
    Vec(Vec<u8>),
}

impl<'a> From<&'a [u8]> for Buffer<'a> {
    fn from(value: &'a [u8]) -> Self {
        Buffer::Ref(value)
    }
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
#[derive(Debug)]
pub enum NetOutgoing<'a> {
    #[cfg(feature = "udp")]
    UdpListen { addr: SocketAddr, reuse: bool },
    #[cfg(feature = "udp")]
    UdpUnlisten { slot: usize },
    #[cfg(feature = "udp")]
    UdpPacket {
        slot: usize,
        to: SocketAddr,
        data: Buffer<'a>,
    },
    #[cfg(feature = "udp")]
    UdpPackets {
        slot: usize,
        to: Vec<SocketAddr>,
        data: Buffer<'a>,
    },
    #[cfg(feature = "tun-tap")]
    TunBind { fd: TunFd },
    #[cfg(feature = "tun-tap")]
    TunUnbind { slot: usize },
    #[cfg(feature = "tun-tap")]
    TunPacket { slot: usize, data: Buffer<'a> },
}

/// Represents an input event for a task.
#[derive(Debug)]
pub enum TaskInput<'a, Ext, ChannelId, Event> {
    Net(NetIncoming<'a>),
    Bus(ChannelId, Event),
    Ext(Ext),
}

impl<'a, Ext, ChannelId, Event> TaskInput<'a, Ext, ChannelId, Event> {
    pub fn convert_into<IExt, IChannelId, IEvent>(
        self,
    ) -> Option<TaskInput<'a, IExt, IChannelId, IEvent>>
    where
        Ext: TryInto<IExt>,
        ChannelId: TryInto<IChannelId>,
        Event: TryInto<IEvent>,
    {
        match self {
            TaskInput::Net(net) => Some(TaskInput::Net(net)),
            TaskInput::Bus(channel, event) => Some(TaskInput::Bus(
                channel.try_into().ok()?,
                event.try_into().ok()?,
            )),
            TaskInput::Ext(ext) => Some(TaskInput::Ext(ext.try_into().ok()?)),
        }
    }
}

/// Represents an output event for a task.
#[derive(Debug)]
pub enum TaskOutput<'a, ExtOut, ChannelIn, ChannelOut, Event> {
    Net(NetOutgoing<'a>),
    Bus(BusEvent<ChannelIn, ChannelOut, Event>),
    Ext(ExtOut),
    Destroy,
}

impl<'a, ExtOut, ChannelIn, ChannelOut, Event>
    TaskOutput<'a, ExtOut, ChannelIn, ChannelOut, Event>
{
    pub fn convert_into<
        NExtOut: From<ExtOut>,
        NChannelIn: From<ChannelIn>,
        NChannelOut: From<ChannelOut>,
        NEvent: From<Event>,
    >(
        self,
    ) -> TaskOutput<'a, NExtOut, NChannelIn, NChannelOut, NEvent> {
        match self {
            TaskOutput::Net(net) => TaskOutput::Net(match net {
                #[cfg(feature = "udp")]
                NetOutgoing::UdpListen { addr, reuse } => NetOutgoing::UdpListen { addr, reuse },
                #[cfg(feature = "udp")]
                NetOutgoing::UdpUnlisten { slot } => NetOutgoing::UdpUnlisten { slot },
                #[cfg(feature = "udp")]
                NetOutgoing::UdpPacket { slot, to, data } => {
                    NetOutgoing::UdpPacket { slot, to, data }
                }
                #[cfg(feature = "udp")]
                NetOutgoing::UdpPackets { slot, to, data } => {
                    NetOutgoing::UdpPackets { slot, to, data }
                }
                #[cfg(feature = "tun-tap")]
                NetOutgoing::TunBind { fd } => NetOutgoing::TunBind { fd },
                #[cfg(feature = "tun-tap")]
                NetOutgoing::TunUnbind { slot } => NetOutgoing::TunUnbind { slot },
                #[cfg(feature = "tun-tap")]
                NetOutgoing::TunPacket { slot, data } => NetOutgoing::TunPacket { slot, data },
            }),
            TaskOutput::Bus(event) => TaskOutput::Bus(event.convert_into()),
            TaskOutput::Ext(ext) => TaskOutput::Ext(ext.into()),
            TaskOutput::Destroy => TaskOutput::Destroy,
        }
    }
}

/// Represents a task.
pub trait Task<ExtIn, ExtOut, ChannelIn, ChannelOut, EventIn, EventOut> {
    /// The type identifier for the task.
    const TYPE: u16;

    /// Called each time the task is ticked. Default is 1ms.
    fn on_tick<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskOutput<'a, ExtOut, ChannelIn, ChannelOut, EventOut>>;

    /// Called when an input event is received for the task.
    fn on_event<'a>(
        &mut self,
        now: Instant,
        input: TaskInput<'a, ExtIn, ChannelIn, EventIn>,
    ) -> Option<TaskOutput<'a, ExtOut, ChannelIn, ChannelOut, EventOut>>;

    /// Retrieves the next output event from the task.
    fn pop_output<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskOutput<'a, ExtOut, ChannelIn, ChannelOut, EventOut>>;

    /// Gracefully shuts down the task.
    fn shutdown<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskOutput<'a, ExtOut, ChannelIn, ChannelOut, EventOut>>;
}
