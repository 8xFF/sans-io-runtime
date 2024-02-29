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
//! backend.on_action(Owner::worker(0), NetOutgoing::UdpListen(SocketAddr::from(([127, 0, 0, 1], 0))));
//!
//! let mut buf = [0; 1500];
//!
//! // Process incoming packets
//! if let Some((incoming, owner)) = backend.pop_incoming(Duration::from_secs(1), &mut buf) {
//!     match incoming {
//!         BackendIncoming::UdpPacket { from, to, len } => {
//!             // Handle incoming UDP packet
//!         }
//!         BackendIncoming::UdpListenResult { bind, result } => {
//!             // Handle UDP listen result
//!         }
//!     }
//! }
//!
//! // Send an outgoing UDP packet
//! let from = SocketAddr::from(([127, 0, 0, 1], 1000));
//! let to = SocketAddr::from(([127, 0, 0, 1], 2000));
//! let data = Buffer::Ref(b"hello");
//! backend.on_action(Owner::worker(0), NetOutgoing::UdpPacket { from, to, data });
//!
//! // Unregister an owner and remove associated sockets
//! backend.remove_owner(Owner::worker(0));
//! ```
//!
//! Note: This module assumes that the sans-io-runtime crate and the Mio library are already imported and available.
use std::{net::SocketAddr, time::Duration, usize};

use mio::{net::UdpSocket, Events, Interest, Poll, Token};

use crate::{
    backend::BackendOwner, collections::DynamicDeque, owner::Owner, ErrorDebugger2, NetOutgoing,
};

use super::{Backend, BackendIncoming};

const QUEUE_PKT_NUM: usize = 512;

fn addr_to_token(addr: SocketAddr) -> Token {
    Token(addr.port() as usize)
}

struct UdpContainer<Owner> {
    socket: UdpSocket,
    local_addr: SocketAddr,
    owner: Owner,
}

#[derive(Debug)]
enum InQueue<Owner> {
    UdpListenResult {
        owner: Owner,
        bind: SocketAddr,
        result: Result<SocketAddr, std::io::Error>,
    },
}

pub struct MioBackend<const SOCKET_LIMIT: usize, const STACK_QUEUE_SIZE: usize> {
    poll: Poll,
    event_buffer: Events,
    event_buffer2: heapless::Deque<Token, QUEUE_PKT_NUM>,
    udp_sockets: heapless::FnvIndexMap<Token, UdpContainer<Owner>, SOCKET_LIMIT>,
    output: DynamicDeque<InQueue<Owner>, STACK_QUEUE_SIZE>,
}

impl<const SOCKET_LIMIT: usize, const STACK_QUEUE_SIZE: usize>
    MioBackend<SOCKET_LIMIT, STACK_QUEUE_SIZE>
{
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
                if let Some(socket) = self.udp_sockets.get(token) {
                    if let Ok((buffer_size, addr)) = socket.socket.recv_from(buf) {
                        return Some((
                            BackendIncoming::UdpPacket {
                                from: addr,
                                to: socket.local_addr,
                                len: buffer_size,
                            },
                            socket.owner,
                        ));
                    }
                } else {
                    self.event_buffer2.pop_front();
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
        Self {
            poll: Poll::new().expect("should create mio-poll"),
            event_buffer: Events::with_capacity(QUEUE_PKT_NUM),
            event_buffer2: heapless::Deque::new(),
            udp_sockets: heapless::FnvIndexMap::new(),
            output: DynamicDeque::new(),
        }
    }
}

impl<const SOCKET_LIMIT: usize, const STACK_QUEUE_SIZE: usize> Backend
    for MioBackend<SOCKET_LIMIT, STACK_QUEUE_SIZE>
{
    fn pop_incoming(
        &mut self,
        timeout: Duration,
        buf: &mut [u8],
    ) -> Option<(BackendIncoming, Owner)> {
        if let Some(e) = self.pop_cached_event(buf) {
            return Some(e);
        }

        if let Err(e) = self.poll.poll(&mut self.event_buffer, Some(timeout)) {
            log::error!("Mio poll error {:?}", e);
            return None;
        }

        for event in self.event_buffer.iter() {
            self.event_buffer2
                .push_back(event.token())
                .expect("Should not full");
        }

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
            NetOutgoing::UdpListen(addr) => {
                assert!(
                    !addr.ip().is_unspecified(),
                    "should not bind to unspecified ip"
                );
                log::info!("MioBackend: UdpListen {:?}", addr);
                match UdpSocket::bind(addr) {
                    Ok(mut socket) => {
                        let local_addr = socket.local_addr().expect("should access udp local_addr");
                        let token = addr_to_token(local_addr);
                        if let Err(e) =
                            self.poll
                                .registry()
                                .register(&mut socket, token, Interest::READABLE)
                        {
                            log::error!("Mio register error {:?}", e);
                            return;
                        }
                        self.output.push_back_safe(InQueue::UdpListenResult {
                            owner,
                            bind: addr,
                            result: Ok(local_addr),
                        });
                        self.udp_sockets
                            .insert(
                                token,
                                UdpContainer {
                                    socket,
                                    local_addr,
                                    owner,
                                },
                            )
                            .print_err2("MioBackend udp_sockets full");
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
            NetOutgoing::UdpPacket { from, to, data } => {
                let token = addr_to_token(from);
                if let Some(socket) = self.udp_sockets.get_mut(&token) {
                    if let Err(e) = socket.socket.send_to(&data, to) {
                        log::error!("Mio send_to error {:?}", e);
                    }
                } else {
                    log::error!("Mio send_to error: no socket for {:?}", from);
                }
            }
        }
    }

    fn remove_owner(&mut self, owner: Owner) {
        // remove all sockets owned by owner and unregister from poll
        let mut tokens = Vec::new();
        for (token, socket) in self.udp_sockets.iter_mut() {
            if socket.owner == owner {
                if let Err(e) = self.poll.registry().deregister(&mut socket.socket) {
                    log::error!("Mio deregister error {:?}", e);
                }
                tokens.push(*token);
            }
        }
        for token in tokens {
            self.udp_sockets.remove(&token);
        }
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
        let mut addr2 = None;

        let mut buf = [0; 1500];

        backend.on_action(
            Owner::worker(1),
            NetOutgoing::UdpListen(SocketAddr::from(([127, 0, 0, 1], 0))),
        );
        match backend.pop_incoming(Duration::from_secs(1), &mut buf) {
            Some((BackendIncoming::UdpListenResult { bind, result }, owner)) => {
                assert_eq!(owner, Owner::worker(1));
                assert_eq!(bind, SocketAddr::from(([127, 0, 0, 1], 0)));
                addr1 = Some(result.expect("Expected Ok"));
            }
            _ => panic!("Expected UdpListenResult"),
        }

        backend.on_action(
            Owner::worker(2),
            NetOutgoing::UdpListen(SocketAddr::from(([127, 0, 0, 1], 0))),
        );
        match backend.pop_incoming(Duration::from_secs(1), &mut buf) {
            Some((BackendIncoming::UdpListenResult { bind, result }, owner)) => {
                assert_eq!(owner, Owner::worker(2));
                assert_eq!(bind, SocketAddr::from(([127, 0, 0, 1], 0)));
                addr2 = Some(result.expect("Expected Ok"));
            }
            _ => panic!("Expected UdpListenResult"),
        }

        assert_ne!(addr1, addr2);
        backend.on_action(
            Owner::worker(1),
            NetOutgoing::UdpPacket {
                from: addr1.expect(""),
                to: addr2.expect(""),
                data: Buffer::Ref(b"hello"),
            },
        );

        match backend.pop_incoming(Duration::from_secs(1), &mut buf) {
            Some((BackendIncoming::UdpPacket { from, to, len }, owner)) => {
                assert_eq!(owner, Owner::worker(2));
                assert_eq!(from, addr1.expect(""));
                assert_eq!(to, addr2.expect(""));
                assert_eq!(&buf[0..len], b"hello");
            }
            _ => panic!("Expected UdpPacket"),
        }

        backend.remove_owner(Owner::worker(1));
        assert_eq!(backend.udp_sockets.len(), 1);

        backend.remove_owner(Owner::worker(2));
        assert_eq!(backend.udp_sockets.len(), 0);
    }
}
