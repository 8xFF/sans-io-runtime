use std::{
    net::{SocketAddr, UdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
    usize,
};

use polling::{Event, Events, PollMode, Poller};
use socket2::{Domain, Protocol, Socket, Type};
#[cfg(feature = "tun-tap")]
use std::io::{Read, Write};

use crate::{
    backend::BackendOwner,
    collections::{DynamicDeque, DynamicVec},
    owner::Owner,
    NetOutgoing,
};

use super::{Awaker, Backend, BackendIncoming};

const QUEUE_PKT_NUM: usize = 512;

type Token = usize;

enum SocketType {
    Waker(),
    #[cfg(feature = "udp")]
    #[allow(dead_code)]
    Udp(UdpSocket, SocketAddr, Owner),
    #[cfg(feature = "tun-tap")]
    Tun(super::tun::TunFd, Owner),
}

pub struct PollingBackend<const SOCKET_STACK_SIZE: usize, const QUEUE_STACK_SIZE: usize> {
    poll: Arc<Poller>,
    event_buffer: Events,
    event_buffer2: heapless::Deque<Token, QUEUE_PKT_NUM>,
    sockets: DynamicVec<Option<SocketType>, SOCKET_STACK_SIZE>,
    output: DynamicDeque<(BackendIncoming, Owner), QUEUE_STACK_SIZE>,
    cycle_count: u64,
    awaker: Arc<PollingAwaker>,
    awake_flag: Arc<AtomicBool>,
}

impl<const SOCKET_STACK_SIZE: usize, const QUEUE_STACK_SIZE: usize>
    PollingBackend<SOCKET_STACK_SIZE, QUEUE_STACK_SIZE>
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
    fn pop_cached_event(&mut self, buf: &mut [u8]) -> Option<(BackendIncoming, Owner)> {
        if let Some(wait) = self.output.pop_front() {
            return Some(wait);
        }
        while !self.event_buffer2.is_empty() {
            if let Some(token) = self.event_buffer2.front() {
                let slot_index = *token;
                match self.sockets.get_mut(slot_index) {
                    #[cfg(feature = "udp")]
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
                    #[cfg(feature = "tun-tap")]
                    Some(Some(SocketType::Tun(fd, owner))) => {
                        if fd.read {
                            if let Ok(size) = fd.fd.read(buf) {
                                return Some((
                                    BackendIncoming::TunPacket {
                                        slot: slot_index,
                                        len: size,
                                    },
                                    *owner,
                                ));
                            }
                        }
                        self.event_buffer2.pop_front();
                    }
                    Some(None) | None => {
                        self.event_buffer2.pop_front();
                    }
                }
            }
        }

        None
    }
}

impl<const SOCKET_LIMIT: usize, const STACK_QUEUE_SIZE: usize> Default
    for PollingBackend<SOCKET_LIMIT, STACK_QUEUE_SIZE>
{
    fn default() -> Self {
        let poll = Arc::new(Poller::new().expect("should create mio-poll"));
        let awake_flag = Arc::new(AtomicBool::new(false));
        Self {
            poll: poll.clone(),
            event_buffer: Events::new(),
            event_buffer2: heapless::Deque::new(),
            sockets: DynamicVec::from([Some(SocketType::Waker())]),
            output: DynamicDeque::default(),
            cycle_count: 0,
            awake_flag: awake_flag.clone(),
            awaker: Arc::new(PollingAwaker { poll, awake_flag }),
        }
    }
}

impl<const SOCKET_LIMIT: usize, const STACK_QUEUE_SIZE: usize> Backend
    for PollingBackend<SOCKET_LIMIT, STACK_QUEUE_SIZE>
{
    fn create_awaker(&self) -> Arc<dyn Awaker> {
        self.awaker.clone()
    }

    fn poll_incoming(&mut self, timeout: Duration) {
        self.cycle_count += 1;
        if let Err(e) = self.poll.wait(&mut self.event_buffer, Some(timeout)) {
            log::error!("Polling poll error {:?}", e);
            return;
        }

        if self.awake_flag.load(Ordering::Relaxed) {
            self.awake_flag.store(false, Ordering::Relaxed);
            self.event_buffer2.push_back(0).expect("Should not full");
        }

        for event in self.event_buffer.iter() {
            self.event_buffer2
                .push_back(event.key)
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
    for PollingBackend<SOCKET_LIMIT, QUEUE_SIZE>
{
    fn on_action(&mut self, owner: Owner, action: NetOutgoing) {
        match action {
            #[cfg(feature = "udp")]
            NetOutgoing::UdpListen { addr, reuse } => {
                log::info!("PollingBackend: UdpListen {addr}, reuse: {reuse}");
                match Self::create_udp(addr, reuse) {
                    Ok(socket) => {
                        let local_addr = socket.local_addr().expect("should access udp local_addr");
                        let slot = self.select_slot();
                        unsafe {
                            if let Err(e) = self.poll.add_with_mode(
                                &socket,
                                Event::readable(slot),
                                PollMode::Level,
                            ) {
                                log::error!("Polling register error {:?}", e);
                                return;
                            }
                        }

                        self.output.push_back_safe((
                            BackendIncoming::UdpListenResult {
                                bind: addr,
                                result: Ok((local_addr, slot)),
                            },
                            owner,
                        ));
                        *self.sockets.get_mut_or_panic(slot) =
                            Some(SocketType::Udp(socket, local_addr, owner));
                    }
                    Err(e) => {
                        log::error!("Polling bind error {:?}", e);
                        self.output.push_back_safe((
                            BackendIncoming::UdpListenResult {
                                bind: addr,
                                result: Err(e),
                            },
                            owner,
                        ));
                    }
                }
            }
            #[cfg(feature = "udp")]
            NetOutgoing::UdpUnlisten { slot } => {
                if let Some(slot) = self.sockets.get_mut(slot) {
                    if let Some(SocketType::Udp(socket, _, _)) = slot {
                        if let Err(e) = self.poll.delete(socket) {
                            log::error!("Polling deregister error {:?}", e);
                        }
                        *slot = None;
                    }
                }
            }
            #[cfg(feature = "udp")]
            NetOutgoing::UdpPacket { to, slot, data } => {
                if let Some(socket) = self.sockets.get_mut(slot) {
                    if let Some(SocketType::Udp(socket, _, _)) = socket {
                        if let Err(e) = socket.send_to(&data, to) {
                            log::error!("Polling send_to error {:?}", e);
                        }
                    } else {
                        log::error!("Polling send_to error: no socket for {:?}", to);
                    }
                } else {
                    log::error!("Polling send_to error: no socket for {:?}", to);
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
                use std::os::unix::io::AsRawFd;

                let slot = self.select_slot();
                if fd.read {
                    unsafe {
                        if let Err(e) = self.poll.add_with_mode(
                            fd.fd.as_raw_fd(),
                            Event::readable(slot),
                            PollMode::Level,
                        ) {
                            log::error!("Polling register error {:?}", e);
                            return;
                        }
                    }
                }

                self.output
                    .push_back_safe((BackendIncoming::TunBindResult { result: Ok(slot) }, owner));
                *self.sockets.get_mut_or_panic(slot) = Some(SocketType::Tun(fd, owner));
            }
            #[cfg(feature = "tun-tap")]
            NetOutgoing::TunUnbind { slot } => {
                use std::os::fd::BorrowedFd;
                use std::os::unix::io::AsRawFd;

                if let Some(slot) = self.sockets.get_mut(slot) {
                    if let Some(SocketType::Tun(fd, _)) = slot {
                        if fd.read {
                            let fd = unsafe { BorrowedFd::borrow_raw(fd.fd.as_raw_fd()) };
                            if let Err(e) = self.poll.delete(fd) {
                                log::error!("Polling deregister error {:?}", e);
                            }
                        }
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
                Some(SocketType::Udp(socket, _, owner2)) => {
                    if *owner2 == owner {
                        if let Err(e) = self.poll.delete(socket) {
                            log::error!("Polling deregister error {:?}", e);
                        }
                        *slot = None;
                    }
                }
                #[cfg(feature = "tun-tap")]
                Some(SocketType::Tun(fd, owner2)) => {
                    use std::os::fd::BorrowedFd;
                    use std::os::unix::io::AsRawFd;

                    if *owner2 == owner {
                        if fd.read {
                            let fd = unsafe { BorrowedFd::borrow_raw(fd.fd.as_raw_fd()) };
                            if let Err(e) = self.poll.delete(fd) {
                                log::error!("Polling deregister error {:?}", e);
                            }
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

pub struct PollingAwaker {
    poll: Arc<Poller>,
    awake_flag: Arc<AtomicBool>,
}

impl Awaker for PollingAwaker {
    fn awake(&self) {
        if self.awake_flag.load(Ordering::Relaxed) {
            return;
        }
        self.awake_flag.store(true, Ordering::Relaxed);
        self.poll.notify().expect("Should notify poll");
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

    use super::PollingBackend;

    #[allow(unused_assignments)]
    #[cfg(feature = "udp")]
    #[test]
    fn test_on_action_udp_listen_success() {
        let mut backend = PollingBackend::<2, 2>::default();

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
        backend.poll_incoming(Duration::from_millis(100));
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
        backend.poll_incoming(Duration::from_millis(100));
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

        backend.poll_incoming(Duration::from_millis(100));
        for _ in 0..10 {
            match backend.pop_incoming(&mut buf) {
                Some((BackendIncoming::UdpPacket { from, slot, len }, owner)) => {
                    assert_eq!(owner, Owner::worker(2));
                    assert_eq!(from, addr1.expect(""));
                    assert_eq!(slot, slot2);
                    assert_eq!(&buf[0..len], b"hello");
                    break;
                }
                _ => {}
            }
        }

        backend.remove_owner(Owner::worker(1));
        assert_eq!(backend.socket_count(), 2);

        backend.remove_owner(Owner::worker(2));
        assert_eq!(backend.socket_count(), 1);
    }
}
