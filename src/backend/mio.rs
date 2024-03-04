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
//!         _ => { }
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
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::SocketAddr,
    time::Duration,
    usize,
};

use mio::{
    net::{TcpListener, TcpStream, UdpSocket},
    Events, Interest, Poll, Token,
};

use crate::{
    backend::BackendOwner, collections::DynamicDeque, owner::Owner, ErrorDebugger2, NetOutgoing,
};

use super::{Backend, BackendIncoming};

const QUEUE_PKT_NUM: usize = 512;

fn addr_to_token(addr: SocketAddr) -> Token {
    Token(addr.port() as usize)
}

fn addr_seed_to_token(addr: SocketAddr, seed: u16) -> Token {
    let val = ((seed as usize) << 16) | (addr.port() as usize);
    Token(val)
}

fn reserve_seed_addr_from_token(token: Token) -> (u16, Token) {
    let port: u16 = token.0 as u16;
    let seed = ((token.0) >> 16) as u16;
    (seed, Token(port as usize))
}

struct TcpStreamContainer<Owner> {
    stream: TcpStream,
    addr: SocketAddr,
    owner: Owner,
}

struct TcpListenerContainer<Owner> {
    listener: TcpListener,
    local_addr: SocketAddr,
    owner: Owner,
    conns: HashMap<u16, TcpStreamContainer<Owner>>,
    conn_addrs: HashMap<SocketAddr, u16>,
    seed: u16,
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
    TcpListenerResult {
        owner: Owner,
        bind: SocketAddr,
        result: Result<SocketAddr, std::io::Error>,
    },
    TcpOnDisconnected {
        owner: Owner,
        listener_token: Token,
        seed: u16,
    },
}

pub struct MioBackend<const SOCKET_LIMIT: usize, const STACK_QUEUE_SIZE: usize> {
    poll: Poll,
    event_buffer: Events,
    event_buffer2: heapless::Deque<Token, QUEUE_PKT_NUM>,
    udp_sockets: heapless::FnvIndexMap<Token, UdpContainer<Owner>, SOCKET_LIMIT>,
    tcp_listeners: heapless::FnvIndexMap<Token, TcpListenerContainer<Owner>, SOCKET_LIMIT>,
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
                InQueue::TcpListenerResult {
                    owner,
                    bind,
                    result,
                } => {
                    return Some((BackendIncoming::TcpListenResult { bind, result }, owner));
                }
                InQueue::TcpOnDisconnected {
                    owner,
                    listener_token,
                    seed,
                } => {
                    if let Some(listener) = self.tcp_listeners.get_mut(&listener_token) {
                        if let Some(mut conn) = listener.conns.remove(&seed) {
                            let _ = self.poll.registry().deregister(&mut conn.stream);
                            listener.conn_addrs.remove(&conn.addr);
                            return Some((
                                BackendIncoming::TcpOnDisconnected {
                                    local_addr: listener.local_addr,
                                    remote_addr: conn.addr,
                                },
                                owner,
                            ));
                        }
                    }
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
                    } else {
                        self.event_buffer2.pop_front();
                    }
                } else if let Some(listener) = self.tcp_listeners.get_mut(token) {
                    if let Ok((mut conn, addr)) = listener.listener.accept() {
                        let token = addr_seed_to_token(listener.local_addr, listener.seed);
                        if let Err(e) = self.poll.registry().register(
                            &mut conn,
                            token,
                            Interest::READABLE.add(Interest::WRITABLE),
                        ) {
                            log::error!("Mio register error {:?}", e);
                            return None;
                        }

                        listener.conns.insert(
                            listener.seed,
                            TcpStreamContainer {
                                stream: conn,
                                addr,
                                owner: listener.owner,
                            },
                        );
                        listener.conn_addrs.insert(addr, listener.seed);
                        listener.seed += 1;
                        return Some((
                            BackendIncoming::TcpOnConnected {
                                local_addr: listener.local_addr,
                                remote_addr: addr,
                            },
                            listener.owner,
                        ));
                    } else {
                        self.event_buffer2.pop_front();
                    }
                } else {
                    let (seed, listener_token) = reserve_seed_addr_from_token(*token);
                    if seed > 0 {
                        if let Some(listener) = self.tcp_listeners.get_mut(&listener_token) {
                            if let Some(conn) = listener.conns.get_mut(&seed) {
                                if let Ok(n) = conn.stream.read(buf) {
                                    if n == 0 {
                                        self.output.push_back_safe(InQueue::TcpOnDisconnected {
                                            owner: conn.owner,
                                            listener_token,
                                            seed,
                                        });
                                    } else {
                                        return Some((
                                            BackendIncoming::TcpPacket {
                                                from: conn.addr,
                                                to: listener.local_addr,
                                                len: n,
                                            },
                                            conn.owner,
                                        ));
                                    }
                                }
                            }
                        }
                    }
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
            tcp_listeners: heapless::FnvIndexMap::new(),
            output: DynamicDeque::default(),
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
            NetOutgoing::TcpListen(addr) => {
                assert!(
                    !addr.ip().is_unspecified(),
                    "should not bind to unspecified ip"
                );
                log::info!("MioBackend: TcpListen {:?}", addr);
                match TcpListener::bind(addr) {
                    Ok(mut listener) => {
                        let local_addr =
                            listener.local_addr().expect("should access udp local_addr");
                        let token = addr_to_token(local_addr);
                        if let Err(e) =
                            self.poll
                                .registry()
                                .register(&mut listener, token, Interest::READABLE)
                        {
                            log::error!("Mio register error {:?}", e);
                            return;
                        }
                        self.output.push_back_safe(InQueue::TcpListenerResult {
                            owner,
                            bind: addr,
                            result: Ok(local_addr),
                        });
                        self.tcp_listeners
                            .insert(
                                token,
                                TcpListenerContainer {
                                    listener,
                                    local_addr,
                                    owner,
                                    seed: 1,
                                    conns: HashMap::new(),
                                    conn_addrs: HashMap::new(),
                                },
                            )
                            .print_err2("MioBackend tcp_listener full");
                    }
                    Err(e) => {
                        log::error!("MioBackend: Tcp bind error {:?}", e);
                        self.output.push_back_safe(InQueue::TcpListenerResult {
                            owner,
                            bind: addr,
                            result: Err(e),
                        })
                    }
                }
            }
            NetOutgoing::TcpPacket { from, to, data } => {
                let token = addr_to_token(from);
                if let Some(listener) = self.tcp_listeners.get_mut(&token) {
                    if let Some(seed) = listener.conn_addrs.get(&to) {
                        if let Some(conn) = listener.conns.get_mut(seed) {
                            if let Err(e) = conn.stream.write(&data) {
                                log::error!("Mio send_to error {:?}", e);
                            }
                        } else {
                            log::error!("Mio not found connection for seed {}", seed);
                        }
                    } else {
                        log::error!("Mio not found connection for addr {}", to);
                    }
                } else {
                    log::error!("Mio not found connection for addr {}", to);
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
    use std::{io::Write, net::SocketAddr, time::Duration};

    use mio::net::TcpStream;

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

    #[allow(unused_assignments)]
    #[test]
    fn test_on_action_tcp_listener_success() {
        let mut backend = MioBackend::<2, 2>::default();
        let server_addr = SocketAddr::from(([127, 0, 0, 1], 8080));
        let mut buf = [0; 1500];

        backend.on_action(
            Owner::worker(1),
            NetOutgoing::TcpListen(SocketAddr::from(([127, 0, 0, 1], 8080))),
        );

        match backend.pop_incoming(Duration::from_secs(1), &mut buf) {
            Some((BackendIncoming::TcpListenResult { bind, .. }, owner)) => {
                assert_eq!(owner, Owner::worker(1));
                assert_eq!(bind, SocketAddr::from(([127, 0, 0, 1], 8080)));
            }
            _ => panic!("Expected TcpListenerResult"),
        }

        let client = TcpStream::connect(server_addr);
        match client {
            Ok(mut stream) => {
                let mut cli_addr = None;
                match backend.pop_incoming(Duration::from_secs(1), &mut buf) {
                    Some((
                        BackendIncoming::TcpOnConnected {
                            local_addr,
                            remote_addr,
                        },
                        owner,
                    )) => {
                        assert_eq!(owner, Owner::worker(1));
                        assert_eq!(local_addr, SocketAddr::from(([127, 0, 0, 1], 8080)));
                        cli_addr = Some(remote_addr);
                    }
                    _ => panic!("Expected TcpOnConnected"),
                };

                let _ = stream.write(b"hello").unwrap();
                let mut tick = 0;

                loop {
                    match backend.pop_incoming(Duration::from_secs(1), &mut buf) {
                        Some((BackendIncoming::TcpPacket { from, to, len }, owner)) => {
                            assert_eq!(owner, Owner::worker(1));
                            assert_eq!(from, cli_addr.expect(""));
                            assert_eq!(to, server_addr);
                            assert_eq!(&buf[0..len], b"hello");
                            break;
                        }
                        _ => {
                            tick += 1;
                            if tick > 30 {
                                panic!("timeout to wait tcp packet");
                            }
                        }
                    }
                }

                tick = 0;
                drop(stream);
                loop {
                    match backend.pop_incoming(Duration::from_secs(1), &mut buf) {
                        Some((
                            BackendIncoming::TcpOnDisconnected {
                                local_addr,
                                remote_addr,
                            },
                            owner,
                        )) => {
                            assert_eq!(owner, Owner::worker(1));
                            assert_eq!(local_addr, SocketAddr::from(([127, 0, 0, 1], 8080)));
                            assert_eq!(remote_addr, cli_addr.expect(""));
                            break;
                        }
                        _ => {
                            tick += 1;
                            if tick > 30 {
                                panic!("timeout to wait tcp connection close event");
                            }
                        }
                    }
                }
            }
            Err(_) => panic!("connection error"),
        }
    }
}
