use std::{net::SocketAddr, time::Duration, usize};

use mio::{net::UdpSocket, Events, Interest, Poll, Token};

use crate::{
    collections::DynamicDeque, owner::Owner, trace::ErrorDebugger, BackendOwner, ErrorDebugger2,
    NetIncoming, NetOutgoing,
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
