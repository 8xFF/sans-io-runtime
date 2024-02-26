use std::{
    collections::{HashMap, VecDeque},
    io::{self, Read, Write},
    net::SocketAddr,
    time::Duration,
};

use mio::{
    net::{TcpListener, TcpStream, UdpSocket},
    Events, Interest, Poll, Token,
};

use crate::{NetIncoming, NetOutgoing};

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

struct TcpConnectionContainer<Owner> {
    stream: TcpStream,
    remote_addr: SocketAddr,
    owner: Owner,
}

struct TcpContainer<Owner> {
    listener: TcpListener,
    local_addr: SocketAddr,
    conns: HashMap<SocketAddr, TcpConnectionContainer<Owner>>,
    owner: Owner,
}

struct TcpConnectionPath {
    local_addr: SocketAddr,
    remote_addr: SocketAddr,
}

impl TcpConnectionPath {
    fn token(&self) -> Token {
        let value = (self.remote_addr.port() as usize) << 16 | (self.local_addr.port() as usize);
        Token(value)
    }
}

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
    TcpListenResult {
        owner: Owner,
        bind: SocketAddr,
        result: Result<SocketAddr, std::io::Error>,
    },
    TcpNewConnection {
        owner: Owner,
        remote_addr: SocketAddr,
    },
    TcpData {
        owner: Owner,
        from: SocketAddr,
        to: SocketAddr,
        buffer_index: usize,
        buffer_size: usize,
    },
    TcpConnectionClose {
        owner: Owner,
        local_addr: SocketAddr,
        remote_addr: SocketAddr,
    },
}

pub struct MioBackend<Owner: Copy + PartialEq + Eq> {
    poll: Poll,
    event_buffer: Events,
    udp_sockets: HashMap<Token, UdpContainer<Owner>>,
    tcp_listeners: HashMap<Token, TcpContainer<Owner>>,
    connection_paths: HashMap<Token, TcpConnectionPath>,
    output: VecDeque<InQueue<Owner>>,
    buffers: [[u8; 1500]; QUEUE_PKT_NUM],
    buffer_index: usize,
    buffer_inqueue_count: usize,
}

impl<Owner: Copy + PartialEq + Eq> Default for MioBackend<Owner> {
    fn default() -> Self {
        Self {
            poll: Poll::new().expect("should create mio-poll"),
            event_buffer: Events::with_capacity(QUEUE_PKT_NUM),
            udp_sockets: HashMap::new(),
            tcp_listeners: HashMap::new(),
            connection_paths: HashMap::new(),
            output: VecDeque::new(),
            buffers: [[0; 1500]; QUEUE_PKT_NUM],
            buffer_index: 0,
            buffer_inqueue_count: 0,
        }
    }
}

impl<Owner: Copy + PartialEq + Eq> Backend<Owner> for MioBackend<Owner> {
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

    fn swap_owner(&mut self, from: Owner, to: Owner) {
        for (_, socket) in self.udp_sockets.iter_mut() {
            if socket.owner == from {
                socket.owner = to;
            }
        }
    }

    fn on_action(&mut self, action: NetOutgoing, owner: Owner) {
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
                        self.output.push_back(InQueue::UdpListenResult {
                            owner,
                            bind: addr,
                            result: Ok(local_addr),
                        });
                        self.udp_sockets.insert(
                            token,
                            UdpContainer {
                                socket,
                                local_addr,
                                owner,
                            },
                        );
                    }
                    Err(e) => {
                        log::error!("Mio bind error {:?}", e);
                        self.output.push_back(InQueue::UdpListenResult {
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
            NetOutgoing::TcpListen(addr) => {
                assert!(
                    !addr.ip().is_unspecified(),
                    "should not bind to unspecified ip"
                );
                log::info!("MioBackend: TcpListen: {:?}", addr);
                match TcpListener::bind(addr) {
                    Ok(mut listener) => {
                        let local_addr =
                            listener.local_addr().expect("should access tcp local_addr");
                        let token = addr_to_token(local_addr);
                        if let Err(e) =
                            self.poll
                                .registry()
                                .register(&mut listener, token, Interest::READABLE)
                        {
                            log::error!("MioBackend: register tcp listener event error {:?}", e);
                            return;
                        }

                        self.output.push_back(InQueue::TcpListenResult {
                            owner,
                            bind: addr,
                            result: Ok(local_addr),
                        });
                        self.tcp_listeners.insert(
                            token,
                            TcpContainer {
                                listener,
                                local_addr,
                                conns: HashMap::new(),
                                owner,
                            },
                        );
                    }
                    Err(e) => {
                        log::error!("MioBackend: bind error {:?}", e);
                        self.output.push_back(InQueue::TcpListenResult {
                            owner,
                            bind: addr,
                            result: Err(e),
                        });
                    }
                }
            }
            NetOutgoing::TcpPacket { from, to, data } => {
                let token = addr_to_token(from);
                if let Some(listner) = self.tcp_listeners.get_mut(&token) {
                    if let Some(conn) = listner.conns.get_mut(&to) {
                        if let Err(e) = conn.stream.write(data) {
                            log::error!("MioBackend: Tcp send error: {:?}", e);
                        }
                    } else {
                        log::error!("MioBackend: Tcp send error: no connection for {:?}", to);
                    }
                }
            }
            NetOutgoing::TcpClose {
                local_addr,
                remote_addr,
            } => {
                self.output.push_back(InQueue::TcpConnectionClose {
                    owner,
                    local_addr,
                    remote_addr,
                });
            }
        }
    }

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
                    InQueue::TcpListenResult {
                        owner,
                        bind,
                        result,
                    } => {
                        return Some((NetIncoming::TcpListenResult { bind, result }, owner));
                    }
                    InQueue::TcpNewConnection { owner, remote_addr } => {
                        return Some((NetIncoming::TcpNewConnection { remote_addr }, owner));
                    }
                    InQueue::TcpData {
                        owner,
                        from,
                        to,
                        buffer_index,
                        buffer_size,
                    } => {
                        self.buffer_inqueue_count -= 1;
                        return Some((
                            NetIncoming::TcpPacket {
                                from,
                                to,
                                data: &self.buffers[buffer_index][0..buffer_size],
                            },
                            owner,
                        ));
                    }
                    InQueue::TcpConnectionClose {
                        owner,
                        local_addr,
                        remote_addr,
                    } => {
                        let token = addr_to_token(local_addr);
                        if let Some(listener) = self.tcp_listeners.get_mut(&token) {
                            if let Some(mut conn) = listener.conns.remove(&remote_addr) {
                                let _ = self.poll.registry().deregister(&mut conn.stream);
                                return Some((
                                    NetIncoming::TcpConnectionClose {
                                        addr: conn.remote_addr,
                                    },
                                    owner,
                                ));
                            }
                        }
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
                        self.output.push_back(InQueue::UdpData {
                            owner: socket.owner,
                            from: addr,
                            to: socket.local_addr,
                            buffer_index,
                            buffer_size,
                        });
                        self.buffer_inqueue_count += 1;
                        if self.buffer_inqueue_count >= QUEUE_PKT_NUM {
                            log::warn!("Mio backend buffer full");
                            break;
                        }
                    }
                }
                if let Some(listener) = self.tcp_listeners.get_mut(&event.token()) {
                    match listener.listener.accept() {
                        Ok((mut stream, addr)) => {
                            let path = TcpConnectionPath {
                                local_addr: listener.local_addr,
                                remote_addr: addr,
                            };
                            let token = path.token();
                            if let Err(e) = self.poll.registry().register(
                                &mut stream,
                                token,
                                Interest::READABLE.add(Interest::WRITABLE),
                            ) {
                                log::error!(
                                    "MioBackend: error when register tcp stream event {:?}",
                                    e
                                );
                                break;
                            }

                            self.output.push_back(InQueue::TcpNewConnection {
                                owner: listener.owner,
                                remote_addr: addr,
                            });
                            listener.conns.insert(
                                addr,
                                TcpConnectionContainer {
                                    stream,
                                    remote_addr: addr,
                                    owner: listener.owner,
                                },
                            );
                            self.connection_paths.insert(token, path);
                        }
                        Err(e) => {
                            log::error!("MioBackend: accept connection error {:?}", e);
                            break;
                        }
                    }
                }

                if let Some(path) = self.connection_paths.get_mut(&event.token()) {
                    let token = addr_to_token(path.local_addr);
                    if let Some(listener) = self.tcp_listeners.get_mut(&token) {
                        if let Some(conn) = listener.conns.get_mut(&path.remote_addr) {
                            if event.is_readable() {
                                let mut connection_closed = false;
                                let buffer_index = self.buffer_index;
                                loop {
                                    match conn.stream.read(&mut self.buffers[buffer_index]) {
                                        Ok(0) => {
                                            connection_closed = true;
                                            break;
                                        }
                                        Ok(n) => {
                                            self.buffer_index =
                                                (self.buffer_index + 1) % self.buffers.len();
                                            self.output.push_back(InQueue::TcpData {
                                                owner: conn.owner,
                                                from: conn.remote_addr,
                                                to: listener.local_addr,
                                                buffer_index,
                                                buffer_size: n,
                                            });
                                            self.buffer_inqueue_count += 1;
                                            if self.buffer_inqueue_count >= QUEUE_PKT_NUM {
                                                log::warn!("MioBackend: buffer full");
                                                break;
                                            }
                                        }
                                        Err(e) => match e.kind() {
                                            io::ErrorKind::WouldBlock => break,
                                            io::ErrorKind::Interrupted => continue,
                                            _ => {
                                                log::error!(
                                                    "MioBackend: Error when received data from tcp {:?}",
                                                    e
                                                );
                                            }
                                        },
                                    }
                                }

                                if connection_closed {
                                    self.output.push_back(InQueue::TcpConnectionClose {
                                        owner: listener.owner,
                                        local_addr: listener.local_addr,
                                        remote_addr: conn.remote_addr,
                                    });
                                }
                            }
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
