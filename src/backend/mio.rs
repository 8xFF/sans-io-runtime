//! This module contains the implementation of the MioBackend struct, which is a backend for the sans-io-runtime crate.
//!
//! The MioBackend struct provides an implementation of the Backend trait, allowing it to be used as a backend for the sans-io-runtime crate. It uses the Mio library for event-driven I/O operations.
//!
//! The MioBackend struct maintains a collection of UDP sockets, handles incoming and outgoing network packets, and provides methods for registering and unregistering owners.
//!
//! Example usage:
//!
//! ```rust
//! use sans_io_runtime::backend::{Backend, BackendOwner, MioBackend};
//! use sans_io_runtime::{Owner, NetIncoming, NetOutgoing};
//! use std::time::Duration;
//! use std::net::SocketAddr;
//!
//! // Create a MioBackend instance
//! let mut backend = MioBackend::<8, 64>::default();
//!
//! // Register an owner and bind a UDP socket
//! backend.on_action(Owner::worker(0), NetOutgoing::UdpListen(SocketAddr::from(([127, 0, 0, 1], 0))));
//!
//! // Process incoming packets
//! if let Some((incoming, owner)) = backend.pop_incoming(Duration::from_secs(1)) {
//!     match incoming {
//!         NetIncoming::UdpPacket { from, to, data } => {
//!             // Handle incoming UDP packet
//!         }
//!         NetIncoming::UdpListenResult { bind, result } => {
//!             // Handle UDP listen result
//!         }
//!     }
//! }
//!
//! // Send an outgoing UDP packet
//! let from = SocketAddr::from(([127, 0, 0, 1], 1000));
//! let to = SocketAddr::from(([127, 0, 0, 1], 2000));
//! let data = b"hello";
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
    backend::BackendOwner, collections::DynamicDeque, owner::Owner, trace::ErrorDebugger,
    ErrorDebugger2, NetIncoming, NetOutgoing,
};

use super::Backend;

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
    UdpData {
        owner: Owner,
        from: SocketAddr,
        to: SocketAddr,
        buffer_index: usize,
        buffer_size: usize,
    },
    UdpListenResult {
        owner: Owner,
        bind: SocketAddr,
        result: Result<SocketAddr, std::io::Error>,
    },
}

pub struct MioBackend<const SOCKET_LIMIT: usize, const QUEUE_SIZE: usize> {
    poll: Poll,
    event_buffer: Events,
    udp_sockets: heapless::FnvIndexMap<Token, UdpContainer<Owner>, SOCKET_LIMIT>,
    output: DynamicDeque<InQueue<Owner>, QUEUE_SIZE>,
    buffers: [[u8; 1500]; QUEUE_PKT_NUM],
    buffer_index: usize,
    buffer_inqueue_count: usize,
}

impl<const SOCKET_LIMIT: usize, const QUEUE_SIZE: usize> Default
    for MioBackend<SOCKET_LIMIT, QUEUE_SIZE>
{
    fn default() -> Self {
        Self {
            poll: Poll::new().expect("should create mio-poll"),
            event_buffer: Events::with_capacity(QUEUE_PKT_NUM),
            udp_sockets: heapless::FnvIndexMap::new(),
            output: DynamicDeque::new(),
            buffers: [[0; 1500]; QUEUE_PKT_NUM],
            buffer_index: 0,
            buffer_inqueue_count: 0,
        }
    }
}

impl<const SOCKET_LIMIT: usize, const QUEUE_SIZE: usize> Backend
    for MioBackend<SOCKET_LIMIT, QUEUE_SIZE>
{
    fn pop_incoming(&mut self, timeout: Duration) -> Option<(NetIncoming, Owner)> {
        loop {
            if let Some(wait) = self.output.pop_front() {
                match wait {
                    InQueue::UdpData {
                        owner,
                        from,
                        to,
                        buffer_index,
                        buffer_size,
                    } => {
                        self.buffer_inqueue_count -= 1;
                        return Some((
                            NetIncoming::UdpPacket {
                                from,
                                to,
                                data: &self.buffers[buffer_index][0..buffer_size],
                            },
                            owner,
                        ));
                    }
                    InQueue::UdpListenResult {
                        owner,
                        bind,
                        result,
                    } => {
                        return Some((NetIncoming::UdpListenResult { bind, result }, owner));
                    }
                }
            }

            if let Err(e) = self.poll.poll(&mut self.event_buffer, Some(timeout)) {
                log::error!("Mio poll error {:?}", e);
                return None;
            }

            for event in self.event_buffer.iter() {
                if self.buffer_inqueue_count >= QUEUE_PKT_NUM {
                    break;
                }
                if let Some(socket) = self.udp_sockets.get(&event.token()) {
                    let buffer_index = self.buffer_index;
                    while let Ok((buffer_size, addr)) =
                        socket.socket.recv_from(&mut self.buffers[buffer_index])
                    {
                        self.buffer_index = (self.buffer_index + 1) % self.buffers.len();
                        self.output
                            .push_back_stack(InQueue::UdpData {
                                owner: socket.owner,
                                from: addr,
                                to: socket.local_addr,
                                buffer_index,
                                buffer_size,
                            })
                            .print_err("MioBackend output queue full");

                        self.buffer_inqueue_count += 1;
                        if self.buffer_inqueue_count >= QUEUE_PKT_NUM {
                            log::warn!("Mio backend buffer full");
                            break;
                        }
                    }
                }
            }

            if self.output.is_empty() {
                return None;
            }
        }
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
                    if let Err(e) = socket.socket.send_to(data, to) {
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
        backend::{Backend, BackendOwner},
        NetIncoming, NetOutgoing, Owner,
    };

    use super::MioBackend;

    #[test]
    fn test_on_action_udp_listen_success() {
        let mut backend = MioBackend::<2, 2>::default();

        let mut addr1 = None;
        let mut addr2 = None;

        backend.on_action(
            Owner::worker(1),
            NetOutgoing::UdpListen(SocketAddr::from(([127, 0, 0, 1], 0))),
        );
        match backend.pop_incoming(Duration::from_secs(1)) {
            Some((NetIncoming::UdpListenResult { bind, result }, owner)) => {
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
        match backend.pop_incoming(Duration::from_secs(1)) {
            Some((NetIncoming::UdpListenResult { bind, result }, owner)) => {
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
                data: b"hello",
            },
        );

        match backend.pop_incoming(Duration::from_secs(1)) {
            Some((NetIncoming::UdpPacket { from, to, data }, owner)) => {
                assert_eq!(owner, Owner::worker(2));
                assert_eq!(from, addr1.expect(""));
                assert_eq!(to, addr2.expect(""));
                assert_eq!(data, b"hello");
            }
            _ => panic!("Expected UdpPacket"),
        }

        backend.remove_owner(Owner::worker(1));
        assert_eq!(backend.udp_sockets.len(), 1);

        backend.remove_owner(Owner::worker(2));
        assert_eq!(backend.udp_sockets.len(), 0);
    }
}
