//! This module contains the implementation of the PollBackend struct, which is a backend for the sans-io-runtime crate.
//!
//! The PollBackend struct provides an implementation of the Backend trait, allowing it to be used as a backend for the sans-io-runtime crate. It uses the Poll library for event-driven I/O operations.
//!
//! The PollBackend struct maintains a collection of UDP sockets, handles incoming and outgoing network packets, and provides methods for registering and unregistering owners.
//!
//! Example usage:
//!
//! ```rust
//! use sans_io_runtime::backend::{Backend, BackendOwner, PollBackend, BackendIncoming};
//! use sans_io_runtime::{Buffer, NetOutgoing};
//! use std::time::Duration;
//! use std::net::SocketAddr;
//!
//! // Create a PollBackend instance
//! let mut backend = PollBackend::<(), 8, 64>::default();
//!
//! // Register an owner and bind a UDP socket
//! backend.on_action((), NetOutgoing::UdpListen { addr: SocketAddr::from(([127, 0, 0, 1], 0)), reuse: false });
//!
//! let mut buf = [0; 1500];
//!
//! // Process incoming packets
//! backend.poll_incoming(Duration::from_secs(1));
//! if let Some(incoming) = backend.pop_incoming(&mut buf) {
//! }
//!
//! // Send an outgoing UDP packet
//! let slot = 0;
//! let to = SocketAddr::from(([127, 0, 0, 1], 2000));
//! let data = Buffer::from(b"hello".as_slice());
//! backend.on_action((), NetOutgoing::UdpPacket { slot, to, data });
//!
//! // Unregister an owner and remove associated sockets
//! backend.remove_owner(());
//! ```
//!
//! Note: This module assumes that the sans-io-runtime crate and the Poll library are already imported and available.
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    net::{SocketAddr, UdpSocket},
    sync::Arc,
    time::Duration,
    usize,
};

use crate::{
    backend::BackendOwner,
    collections::{DynamicDeque, DynamicVec},
    NetOutgoing,
};

use super::{Awaker, Backend, BackendIncoming, BackendIncomingEvent};

#[cfg(feature = "tun-tap")]
use std::io::{Read, Write};

enum SocketType<Owner> {
    #[cfg(feature = "udp")]
    Udp(UdpSocket, SocketAddr, Owner),
    #[cfg(feature = "tun-tap")]
    Tun(super::tun::TunFd, Owner),
}

pub struct PollBackend<Owner, const SOCKET_STACK_SIZE: usize, const QUEUE_STACK_SIZE: usize> {
    sockets: DynamicVec<Option<SocketType<Owner>>, SOCKET_STACK_SIZE>,
    output: DynamicDeque<BackendIncoming<Owner>, QUEUE_STACK_SIZE>,
    cycle_count: u64,
    last_poll_socket: Option<usize>,
}

impl<Owner, const SOCKET_STACK_SIZE: usize, const QUEUE_STACK_SIZE: usize>
    PollBackend<Owner, SOCKET_STACK_SIZE, QUEUE_STACK_SIZE>
{
    #[cfg(feature = "udp")]
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
        Ok(socket.into())
    }

    pub fn socket_count(&self) -> usize {
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
}

impl<Owner, const SOCKET_LIMIT: usize, const STACK_QUEUE_SIZE: usize> Default
    for PollBackend<Owner, SOCKET_LIMIT, STACK_QUEUE_SIZE>
{
    fn default() -> Self {
        Self {
            sockets: DynamicVec::default(),
            output: DynamicDeque::default(),
            cycle_count: 0,
            last_poll_socket: None,
        }
    }
}

impl<Owner: Clone + Copy + PartialEq, const SOCKET_LIMIT: usize, const STACK_QUEUE_SIZE: usize>
    Backend<Owner> for PollBackend<Owner, SOCKET_LIMIT, STACK_QUEUE_SIZE>
{
    fn create_awaker(&self) -> Arc<dyn Awaker> {
        Arc::new(PollAwaker)
    }

    fn poll_incoming(&mut self, timeout: Duration) {
        if !self.output.is_empty() {
            return;
        }
        self.cycle_count += 1;
        // We need manual awake for now
        self.output.push_back_safe(BackendIncoming::Awake);
        std::thread::sleep(timeout);
    }

    fn pop_incoming(&mut self, buf: &mut [u8]) -> Option<BackendIncoming<Owner>> {
        if let Some(out) = self.output.pop_front() {
            return Some(out);
        }
        if self.sockets.is_empty() {
            return None;
        }

        let mut last_poll_socket = self.last_poll_socket.unwrap_or(0);
        loop {
            if let Some(Some(slot)) = self.sockets.get_mut(last_poll_socket) {
                match slot {
                    #[cfg(feature = "udp")]
                    SocketType::Udp(socket, _addr, owner) => {
                        if let Ok((size, remote)) = socket.recv_from(buf) {
                            return Some(BackendIncoming::Event(
                                *owner,
                                BackendIncomingEvent::UdpPacket {
                                    slot: last_poll_socket,
                                    from: remote,
                                    len: size,
                                },
                            ));
                        }
                    }
                    #[cfg(feature = "tun-tap")]
                    SocketType::Tun(fd, owner) => {
                        if fd.read {
                            if let Ok(size) = fd.fd.read(buf) {
                                return Some(BackendIncoming::Event(
                                    *owner,
                                    BackendIncomingEvent::TunPacket {
                                        slot: last_poll_socket,
                                        len: size,
                                    },
                                ));
                            }
                        }
                    }
                }
            }

            if last_poll_socket == self.sockets.len() - 1 {
                break;
            } else {
                last_poll_socket += 1;
            }
        }
        self.last_poll_socket = None;
        None
    }

    fn finish_outgoing_cycle(&mut self) {}

    fn finish_incoming_cycle(&mut self) {}
}

impl<Owner: Clone + Copy + PartialEq, const SOCKET_LIMIT: usize, const QUEUE_SIZE: usize>
    BackendOwner<Owner> for PollBackend<Owner, SOCKET_LIMIT, QUEUE_SIZE>
{
    fn on_action(&mut self, owner: Owner, action: NetOutgoing) {
        match action {
            #[cfg(feature = "udp")]
            NetOutgoing::UdpListen { addr, reuse } => {
                log::info!("PollBackend: UdpListen {addr}, reuse: {reuse}");
                match Self::create_udp(addr, reuse) {
                    Ok(socket) => {
                        let local_addr = socket.local_addr().expect("should access udp local_addr");
                        let slot = self.select_slot();
                        self.output.push_back_safe(BackendIncoming::Event(
                            owner,
                            BackendIncomingEvent::UdpListenResult {
                                bind: addr,
                                result: Ok((local_addr, slot)),
                            },
                        ));
                        *self.sockets.get_mut_or_panic(slot) =
                            Some(SocketType::Udp(socket, local_addr, owner));
                    }
                    Err(e) => {
                        log::error!("Poll bind error {:?}", e);
                        self.output.push_back_safe(BackendIncoming::Event(
                            owner,
                            BackendIncomingEvent::UdpListenResult {
                                bind: addr,
                                result: Err(e),
                            },
                        ));
                    }
                }
            }
            #[cfg(feature = "udp")]
            NetOutgoing::UdpUnlisten { slot } => {
                if let Some(slot) = self.sockets.get_mut(slot) {
                    if let Some(SocketType::Udp(_socket, _, _)) = slot {
                        *slot = None;
                    }
                }
            }
            #[cfg(feature = "udp")]
            NetOutgoing::UdpPacket { to, slot, data } => {
                if let Some(socket) = self.sockets.get_mut(slot) {
                    if let Some(SocketType::Udp(socket, _, _)) = socket {
                        if let Err(e) = socket.send_to(&data, to) {
                            log::error!("Poll send_to error {:?}", e);
                        }
                    } else {
                        log::error!("Poll send_to error: no socket for {}", slot);
                    }
                } else {
                    log::error!("Poll send_to error: no socket for {}", slot);
                }
            }
            #[cfg(feature = "udp")]
            NetOutgoing::UdpPackets { to, slot, data } => {
                if let Some(socket) = self.sockets.get_mut(slot) {
                    if let Some(SocketType::Udp(socket, _, _)) = socket {
                        for dest in to {
                            if let Err(e) = socket.send_to(&data, dest) {
                                log::error!("Poll send_to error {:?}", e);
                            }
                        }
                    } else {
                        log::error!("Poll send_to error: no socket for {}", slot);
                    }
                } else {
                    log::error!("Poll send_to error: no socket for {}", slot);
                }
            }
            #[cfg(feature = "tun-tap")]
            NetOutgoing::TunBind { fd } => {
                let slot = self.select_slot();
                self.output.push_back_safe(BackendIncoming::Event(
                    owner,
                    BackendIncomingEvent::TunBindResult { result: Ok(slot) },
                ));
                *self.sockets.get_mut_or_panic(slot) = Some(SocketType::Tun(fd, owner));
            }
            #[cfg(feature = "tun-tap")]
            NetOutgoing::TunUnbind { slot } => {
                if let Some(slot) = self.sockets.get_mut(slot) {
                    if let Some(SocketType::Tun(_, _)) = slot {
                        *slot = None;
                    }
                }
            }
            #[cfg(feature = "tun-tap")]
            NetOutgoing::TunPacket { slot, data } => {
                if let Some(socket) = self.sockets.get_mut(slot) {
                    if let Some(SocketType::Tun(fd, _)) = socket {
                        if let Err(e) = fd.fd.write_all(&data) {
                            log::error!("Poll write_all error {:?}", e);
                        }
                    } else {
                        log::error!("Poll send_to error: no tun for {}", slot);
                    }
                } else {
                    log::error!("Poll send_to error: no tun for {}", slot);
                }
            }
        }
    }

    // remove all sockets owned by owner and unregister from poll
    fn remove_owner(&mut self, owner: Owner) {
        for slot in self.sockets.iter_mut() {
            match slot {
                #[cfg(feature = "udp")]
                Some(SocketType::Udp(_socket, _, owner2)) => {
                    if *owner2 == owner {
                        *slot = None;
                    }
                }
                #[cfg(feature = "tun-tap")]
                Some(SocketType::Tun(_fd, owner2)) => {
                    if *owner2 == owner {
                        *slot = None;
                    }
                }
                None => {}
            }
        }
    }
}

pub struct PollAwaker;

impl Awaker for PollAwaker {
    fn awake(&self) {
        //do nothing
    }
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, time::Duration};

    use crate::{
        backend::{Backend, BackendIncoming, BackendOwner},
        group_owner_type, NetOutgoing, TaskGroupOwner,
    };

    use super::PollBackend;

    group_owner_type!(SimpleOwner);

    #[allow(unused_assignments)]
    #[cfg(feature = "udp")]
    #[test]
    fn test_on_action_udp_listen_success() {
        use crate::{backend::BackendIncomingEvent, Buffer};

        let mut backend = PollBackend::<SimpleOwner, 2, 2>::default();

        let mut addr1 = None;
        let mut slot1 = 0;
        let mut addr2 = None;
        let mut slot2 = 0;

        let mut buf = [0; 1500];

        backend.on_action(
            SimpleOwner(1),
            NetOutgoing::UdpListen {
                addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                reuse: false,
            },
        );
        backend.poll_incoming(Duration::from_secs(1));
        let res = backend.pop_incoming(&mut buf);
        match res {
            Some(BackendIncoming::Event(
                owner,
                BackendIncomingEvent::UdpListenResult { bind, result },
            )) => {
                assert_eq!(owner, SimpleOwner(1));
                assert_eq!(bind, SocketAddr::from(([127, 0, 0, 1], 0)));
                let res = result.expect("Expected Ok");
                addr1 = Some(res.0);
                slot1 = res.1;
            }
            _ => panic!("Expected UdpListenResult {:?}", res),
        }

        backend.on_action(
            SimpleOwner(2),
            NetOutgoing::UdpListen {
                addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                reuse: false,
            },
        );
        backend.poll_incoming(Duration::from_secs(1));
        match backend.pop_incoming(&mut buf) {
            Some(BackendIncoming::Event(
                owner,
                BackendIncomingEvent::UdpListenResult { bind, result },
            )) => {
                assert_eq!(owner, SimpleOwner(2));
                assert_eq!(bind, SocketAddr::from(([127, 0, 0, 1], 0)));
                let res = result.expect("Expected Ok");
                addr2 = Some(res.0);
                slot2 = res.1;
            }
            _ => panic!("Expected UdpListenResult"),
        }

        assert_ne!(addr1, addr2);
        backend.on_action(
            SimpleOwner(1),
            NetOutgoing::UdpPacket {
                slot: slot1,
                to: addr2.expect(""),
                data: Buffer::from(b"hello".as_slice()),
            },
        );

        backend.poll_incoming(Duration::from_secs(1));
        match backend.pop_incoming(&mut buf) {
            Some(BackendIncoming::Awake) => {}
            _ => panic!("Expected Awake"),
        }

        match backend.pop_incoming(&mut buf) {
            Some(BackendIncoming::Event(
                owner,
                BackendIncomingEvent::UdpPacket { from, slot, len },
            )) => {
                assert_eq!(owner, SimpleOwner(2));
                assert_eq!(from, addr1.expect(""));
                assert_eq!(slot, slot2);
                assert_eq!(&buf[0..len], b"hello");
            }
            _ => panic!("Expected UdpPacket"),
        }

        backend.remove_owner(SimpleOwner(1));
        assert_eq!(backend.socket_count(), 1);

        backend.remove_owner(SimpleOwner(2));
        assert_eq!(backend.socket_count(), 0);
    }
}
