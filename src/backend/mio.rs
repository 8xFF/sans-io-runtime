//! This module contains the implementation of the MioBackend struct, which is a backend for the sans-io-runtime crate.
//!
//! The MioBackend struct provides an implementation of the Backend trait, allowing it to be used as a backend for the sans-io-runtime crate. It uses the Mio library for event-driven I/O operations.
//!
//! The MioBackend struct maintains a collection of UDP sockets, handles incoming and outgoing network packets, and provides methods for registering and unregistering owners.
//!
//! Example usage:
//!
//! ```rust
//! use sans_io_runtime::backend::{Backend, BackendOwner, MioBackend, BackendIncoming};
//! use sans_io_runtime::{Owner, Buffer, NetOutgoing};
//! use std::time::Duration;
//! use std::net::SocketAddr;
//!
//! // Create a MioBackend instance
//! let mut backend = MioBackend::<8, 64>::default();
//!
//! // Register an owner and bind a UDP socket
//! backend.on_action(Owner::worker(0), NetOutgoing::UdpListen { addr: SocketAddr::from(([127, 0, 0, 1], 0)), reuse: false });
//!
//! let mut buf = [0; 1500];
//!
//! // Process incoming packets
//! backend.poll_incoming(Duration::from_secs(1));
//! if let Some((incoming, owner)) = backend.pop_incoming(&mut buf) {
//!     match incoming {
//!         BackendIncoming::UdpPacket { from, slot, len } => {
//!             // Handle incoming UDP packet
//!         }
//!         BackendIncoming::UdpListenResult { bind, result } => {
//!             // Handle UDP listen result
//!         }
//!         BackendIncoming::Awake => {
//!             // Handle awake event
//!         }
//!     }
//! }
//!
//! // Send an outgoing UDP packet
//! let slot = 0;
//! let to = SocketAddr::from(([127, 0, 0, 1], 2000));
//! let data = Buffer::Ref(b"hello");
//! backend.on_action(Owner::worker(0), NetOutgoing::UdpPacket { slot, to, data });
//!
//! // Unregister an owner and remove associated sockets
//! backend.remove_owner(Owner::worker(0));
//! ```
//!
//! Note: This module assumes that the sans-io-runtime crate and the Mio library are already imported and available.
use std::{net::SocketAddr, sync::Arc, time::Duration, usize};

use mio::{net::UdpSocket, Events, Interest, Poll, Token, Waker};
use socket2::{Domain, Protocol, Socket, Type};

use crate::{
    backend::BackendOwner,
    collections::{DynamicDeque, DynamicVec},
    owner::Owner,
    NetOutgoing,
};

use super::{Awaker, Backend, BackendIncoming};

const QUEUE_PKT_NUM: usize = 512;

enum SocketType {
    Waker(),
    Udp(UdpSocket, SocketAddr, Owner),
}

#[derive(Debug)]
enum InQueue<Owner> {
    UdpListenResult {
        owner: Owner,
        bind: SocketAddr,
        result: Result<(SocketAddr, usize), std::io::Error>,
    },
}

pub struct MioBackend<const SOCKET_STACK_SIZE: usize, const QUEUE_STACK_SIZE: usize> {
    poll: Poll,
    event_buffer: Events,
    event_buffer2: heapless::Deque<Token, QUEUE_PKT_NUM>,
    sockets: DynamicVec<Option<SocketType>, SOCKET_STACK_SIZE>,
    output: DynamicDeque<InQueue<Owner>, QUEUE_STACK_SIZE>,
    cycle_count: u64,
    awaker: Arc<MioAwaker>,
}

impl<const SOCKET_STACK_SIZE: usize, const QUEUE_STACK_SIZE: usize>
    MioBackend<SOCKET_STACK_SIZE, QUEUE_STACK_SIZE>
{
    fn create_udp(addr: SocketAddr, reuse: bool) -> Result<UdpSocket, std::io::Error> {
        let socket = match addr {
            SocketAddr::V4(addr) => {
                let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
                if reuse {
                    socket.set_reuse_address(true)?;
                    socket.set_reuse_port(true)?;
                }
                socket.set_nonblocking(true)?;
                socket.bind(&addr.into())?;
                socket
            }
            SocketAddr::V6(addr) => {
                let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
                if reuse {
                    socket.set_reuse_address(true)?;
                    socket.set_reuse_port(true)?;
                }
                socket.set_nonblocking(true)?;
                socket.bind(&addr.into())?;
                socket
            }
        };
        Ok(UdpSocket::from_std(socket.into()))
    }

    fn socket_count(&self) -> usize {
        self.sockets.iter().filter(|s| s.is_some()).count()
    }

    fn select_slot(&mut self) -> usize {
        for (i, slot) in self.sockets.iter_mut().enumerate() {
            if slot.is_none() {
                return i;
            }
        }

        self.sockets.push_safe(None);
        self.sockets.len() - 1
    }
    fn pop_cached_event(&mut self, buf: &mut [u8]) -> Option<(BackendIncoming, Owner)> {
        if let Some(wait) = self.output.pop_front() {
            match wait {
                InQueue::UdpListenResult {
                    owner,
                    bind,
                    result,
                } => {
                    return Some((BackendIncoming::UdpListenResult { bind, result }, owner));
                }
            }
        }
        while !self.event_buffer2.is_empty() {
            if let Some(token) = self.event_buffer2.front() {
                let slot_index = token.0;
                match self.sockets.get_mut(slot_index) {
                    Some(Some(SocketType::Udp(socket, _, owner))) => {
                        self.event_buffer2.pop_front();
                        if let Ok((buffer_size, addr)) = socket.recv_from(buf) {
                            return Some((
                                BackendIncoming::UdpPacket {
                                    from: addr,
                                    slot: slot_index,
                                    len: buffer_size,
                                },
                                *owner,
                            ));
                        } else {
                            self.event_buffer2.pop_front();
                        }
                    }
                    Some(Some(SocketType::Waker())) => {
                        self.event_buffer2.pop_front();
                        return Some((BackendIncoming::Awake, Owner::worker(0)));
                        //TODO dont use 0
                    }
                    _ => {
                        self.event_buffer2.pop_front();
                    }
                }
            }
        }

        None
    }
}

impl<const SOCKET_LIMIT: usize, const STACK_QUEUE_SIZE: usize> Default
    for MioBackend<SOCKET_LIMIT, STACK_QUEUE_SIZE>
{
    fn default() -> Self {
        let poll = Poll::new().expect("should create mio-poll");
        let waker = Waker::new(poll.registry(), Token(0)).expect("should create mio-waker");
        Self {
            poll,
            event_buffer: Events::with_capacity(QUEUE_PKT_NUM),
            event_buffer2: heapless::Deque::new(),
            sockets: DynamicVec::from([Some(SocketType::Waker())]),
            output: DynamicDeque::default(),
            cycle_count: 0,
            awaker: Arc::new(MioAwaker { waker }),
        }
    }
}

impl<const SOCKET_LIMIT: usize, const STACK_QUEUE_SIZE: usize> Backend
    for MioBackend<SOCKET_LIMIT, STACK_QUEUE_SIZE>
{
    fn create_awaker(&self) -> Arc<dyn Awaker> {
        self.awaker.clone()
    }

    fn poll_incoming(&mut self, timeout: Duration) {
        self.cycle_count += 1;
        if let Err(e) = self.poll.poll(&mut self.event_buffer, Some(timeout)) {
            log::error!("Mio poll error {:?}", e);
            return;
        }

        for event in self.event_buffer.iter() {
            self.event_buffer2
                .push_back(event.token())
                .expect("Should not full");
        }
    }

    fn pop_incoming(&mut self, buf: &mut [u8]) -> Option<(BackendIncoming, Owner)> {
        self.pop_cached_event(buf)
    }

    fn finish_outgoing_cycle(&mut self) {}

    fn finish_incoming_cycle(&mut self) {}
}

impl<const SOCKET_LIMIT: usize, const QUEUE_SIZE: usize> BackendOwner
    for MioBackend<SOCKET_LIMIT, QUEUE_SIZE>
{
    fn on_action(&mut self, owner: Owner, action: NetOutgoing) {
        match action {
            NetOutgoing::UdpListen { addr, reuse } => {
                log::info!("MioBackend: UdpListen {addr}, reuse: {reuse}");
                match Self::create_udp(addr, reuse) {
                    Ok(mut socket) => {
                        let local_addr = socket.local_addr().expect("should access udp local_addr");
                        let slot = self.select_slot();
                        if let Err(e) = self.poll.registry().register(
                            &mut socket,
                            Token(slot),
                            Interest::READABLE,
                        ) {
                            log::error!("Mio register error {:?}", e);
                            return;
                        }

                        self.output.push_back_safe(InQueue::UdpListenResult {
                            owner,
                            bind: addr,
                            result: Ok((local_addr, slot)),
                        });
                        *self.sockets.get_mut_or_panic(slot) =
                            Some(SocketType::Udp(socket, local_addr, owner));
                    }
                    Err(e) => {
                        log::error!("Mio bind error {:?}", e);
                        self.output.push_back_safe(InQueue::UdpListenResult {
                            owner,
                            bind: addr,
                            result: Err(e),
                        });
                    }
                }
            }
            NetOutgoing::UdpUnlisten { slot } => {
                if let Some(slot) = self.sockets.get_mut(slot) {
                    if let Some(SocketType::Udp(socket, _, _)) = slot {
                        if let Err(e) = self.poll.registry().deregister(socket) {
                            log::error!("Mio deregister error {:?}", e);
                        }
                        *slot = None;
                    }
                }
            }
            NetOutgoing::UdpPacket { to, slot, data } => {
                if let Some(socket) = self.sockets.get_mut(slot) {
                    if let Some(SocketType::Udp(socket, _, _)) = socket {
                        if let Err(e) = socket.send_to(&data, to) {
                            log::error!("Mio send_to error {:?}", e);
                        }
                    } else {
                        log::error!("Mio send_to error: no socket for {:?}", to);
                    }
                } else {
                    log::error!("Mio send_to error: no socket for {:?}", to);
                }
            }
        }
    }

    // remove all sockets owned by owner and unregister from poll
    fn remove_owner(&mut self, owner: Owner) {
        for slot in self.sockets.iter_mut() {
            match slot {
                Some(SocketType::Udp(socket, _, owner2)) => {
                    if *owner2 == owner {
                        if let Err(e) = self.poll.registry().deregister(socket) {
                            log::error!("Mio deregister error {:?}", e);
                        }
                        *slot = None;
                    }
                }
                Some(SocketType::Waker()) => {}
                None => {}
            }
        }
    }
}

pub struct MioAwaker {
    waker: Waker,
}

impl Awaker for MioAwaker {
    fn awake(&self) {
        self.waker.wake().expect("should wake mio waker");
    }
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, time::Duration};

    use crate::{
        backend::{Backend, BackendIncoming, BackendOwner},
        task::Buffer,
        NetOutgoing, Owner,
    };

    use super::MioBackend;

    #[allow(unused_assignments)]
    #[test]
    fn test_on_action_udp_listen_success() {
        let mut backend = MioBackend::<2, 2>::default();

        let mut addr1 = None;
        let mut slot1 = 0;
        let mut addr2 = None;
        let mut slot2 = 0;

        let mut buf = [0; 1500];

        backend.on_action(
            Owner::worker(1),
            NetOutgoing::UdpListen {
                addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                reuse: false,
            },
        );
        backend.poll_incoming(Duration::from_secs(1));
        match backend.pop_incoming(&mut buf) {
            Some((BackendIncoming::UdpListenResult { bind, result }, owner)) => {
                assert_eq!(owner, Owner::worker(1));
                assert_eq!(bind, SocketAddr::from(([127, 0, 0, 1], 0)));
                let res = result.expect("Expected Ok");
                addr1 = Some(res.0);
                slot1 = res.1;
            }
            _ => panic!("Expected UdpListenResult"),
        }

        backend.on_action(
            Owner::worker(2),
            NetOutgoing::UdpListen {
                addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                reuse: false,
            },
        );
        backend.poll_incoming(Duration::from_secs(1));
        match backend.pop_incoming(&mut buf) {
            Some((BackendIncoming::UdpListenResult { bind, result }, owner)) => {
                assert_eq!(owner, Owner::worker(2));
                assert_eq!(bind, SocketAddr::from(([127, 0, 0, 1], 0)));
                let res = result.expect("Expected Ok");
                addr2 = Some(res.0);
                slot2 = res.1;
            }
            _ => panic!("Expected UdpListenResult"),
        }

        assert_ne!(addr1, addr2);
        backend.on_action(
            Owner::worker(1),
            NetOutgoing::UdpPacket {
                slot: slot1,
                to: addr2.expect(""),
                data: Buffer::Ref(b"hello"),
            },
        );

        backend.poll_incoming(Duration::from_secs(1));
        match backend.pop_incoming(&mut buf) {
            Some((BackendIncoming::UdpPacket { from, slot, len }, owner)) => {
                assert_eq!(owner, Owner::worker(2));
                assert_eq!(from, addr1.expect(""));
                assert_eq!(slot, slot2);
                assert_eq!(&buf[0..len], b"hello");
            }
            _ => panic!("Expected UdpPacket"),
        }

        backend.remove_owner(Owner::worker(1));
        assert_eq!(backend.socket_count(), 2);

        backend.remove_owner(Owner::worker(2));
        assert_eq!(backend.socket_count(), 1);
    }
}
