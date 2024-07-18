use std::{
    fmt::Debug,
    net::{SocketAddr, UdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use polling::{Event, Events, PollMode, Poller};
use socket2::{Domain, Protocol, Socket, Type};
#[cfg(feature = "tun-tap")]
use std::io::{Read, Write};

use crate::{
    backend::BackendOwner,
    collections::{DynamicDeque, DynamicVec},
    Buffer,
};

use super::{Awaker, Backend, BackendIncoming, BackendIncomingInternal, BackendOutgoing};

const QUEUE_PKT_NUM: usize = 512;

type Token = usize;

enum SocketType<Owner> {
    Waker(),
    #[cfg(feature = "udp")]
    #[allow(dead_code)]
    Udp(UdpSocket, SocketAddr, Owner),
    #[cfg(feature = "tun-tap")]
    Tun(super::tun::TunFd, Owner),
}

pub struct PollingBackend<Owner, const SOCKET_STACK_SIZE: usize, const QUEUE_STACK_SIZE: usize> {
    poll: Arc<Poller>,
    event_buffer: Events,
    event_buffer2: heapless::Deque<Token, QUEUE_PKT_NUM>,
    sockets: DynamicVec<Option<SocketType<Owner>>, SOCKET_STACK_SIZE>,
    output: DynamicDeque<BackendIncomingInternal<Owner>, QUEUE_STACK_SIZE>,
    cycle_count: u64,
    awaker: Arc<PollingAwaker>,
    awake_flag: Arc<AtomicBool>,
}

impl<
        Owner: Debug + Clone + Copy + PartialEq,
        const SOCKET_STACK_SIZE: usize,
        const QUEUE_STACK_SIZE: usize,
    > PollingBackend<Owner, SOCKET_STACK_SIZE, QUEUE_STACK_SIZE>
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

        self.sockets.push(None);
        self.sockets.len() - 1
    }
    fn pop_cached_event(&mut self) -> Option<BackendIncomingInternal<Owner>> {
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
                        let mut buf = Buffer::new(100, 1500); //set padding 100byte for other data appending
                        if let Ok((len, addr)) = socket.recv_from(buf.remain_mut()) {
                            buf.move_back_right(len).expect("Should not overflow");
                            return Some(BackendIncomingInternal::Event(
                                *owner,
                                BackendIncoming::UdpPacket {
                                    from: addr,
                                    slot: slot_index,
                                    data: buf,
                                },
                            ));
                        } else {
                            self.event_buffer2.pop_front();
                        }
                    }
                    Some(Some(SocketType::Waker())) => {
                        self.event_buffer2.pop_front();
                        return Some(BackendIncomingInternal::Awake);
                    }
                    #[cfg(feature = "tun-tap")]
                    Some(Some(SocketType::Tun(fd, owner))) => {
                        if fd.read {
                            let mut buf = Buffer::new(100, 1500); //set padding 100byte for other data appending
                            if let Ok(size) = fd.fd.read(buf.remain_mut()) {
                                buf.move_back_right(size).expect("Should not overflow");
                                return Some(BackendIncomingInternal::Event(
                                    *owner,
                                    BackendIncoming::TunPacket {
                                        slot: slot_index,
                                        data: buf,
                                    },
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

impl<Owner, const SOCKET_LIMIT: usize, const STACK_QUEUE_SIZE: usize> Default
    for PollingBackend<Owner, SOCKET_LIMIT, STACK_QUEUE_SIZE>
{
    fn default() -> Self {
        let poll = Arc::new(Poller::new().expect("should create poll"));
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

impl<
        Owner: Debug + Clone + Copy + PartialEq,
        const SOCKET_LIMIT: usize,
        const STACK_QUEUE_SIZE: usize,
    > Backend<Owner> for PollingBackend<Owner, SOCKET_LIMIT, STACK_QUEUE_SIZE>
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

    fn pop_incoming(&mut self) -> Option<BackendIncomingInternal<Owner>> {
        self.pop_cached_event()
    }

    fn finish_outgoing_cycle(&mut self) {}

    fn finish_incoming_cycle(&mut self) {}
}

impl<
        Owner: Debug + Clone + Copy + PartialEq,
        const SOCKET_LIMIT: usize,
        const QUEUE_SIZE: usize,
    > BackendOwner<Owner> for PollingBackend<Owner, SOCKET_LIMIT, QUEUE_SIZE>
{
    fn on_action(&mut self, owner: Owner, action: BackendOutgoing) {
        match action {
            #[cfg(feature = "udp")]
            BackendOutgoing::UdpListen { addr, reuse } => {
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

                        self.output.push_back(BackendIncomingInternal::Event(
                            owner,
                            BackendIncoming::UdpListenResult {
                                bind: addr,
                                result: Ok((local_addr, slot)),
                            },
                        ));
                        *self.sockets.get_mut_or_panic(slot) =
                            Some(SocketType::Udp(socket, local_addr, owner));
                    }
                    Err(e) => {
                        log::error!("Polling bind error {:?}", e);
                        self.output.push_back(BackendIncomingInternal::Event(
                            owner,
                            BackendIncoming::UdpListenResult {
                                bind: addr,
                                result: Err(e),
                            },
                        ));
                    }
                }
            }
            #[cfg(feature = "udp")]
            BackendOutgoing::UdpUnlisten { slot } => {
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
            BackendOutgoing::UdpPacket { to, slot, data } => {
                if let Some(Some(SocketType::Udp(socket, _, _))) = self.sockets.get_mut(slot) {
                    if let Err(e) = socket.send_to(&data, to) {
                        log::error!("Polling send_to error {:?}", e);
                    }
                } else {
                    log::error!("Polling send_to error: no socket for {:?}", to);
                }
            }
            #[cfg(feature = "udp")]
            BackendOutgoing::UdpPackets { to, slot, data } => {
                if let Some(Some(SocketType::Udp(socket, _, _))) = self.sockets.get_mut(slot) {
                    for dest in to {
                        if let Err(e) = socket.send_to(&data, dest) {
                            log::error!("Poll send_to error {:?}", e);
                        }
                    }
                } else {
                    log::error!("Poll send_to error: no socket for {}", slot);
                }
            }
            #[cfg(feature = "tun-tap")]
            BackendOutgoing::TunBind { fd } => {
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

                self.output.push_back(BackendIncomingInternal::Event(
                    owner,
                    BackendIncoming::TunBindResult { result: Ok(slot) },
                ));
                *self.sockets.get_mut_or_panic(slot) = Some(SocketType::Tun(fd, owner));
            }
            #[cfg(feature = "tun-tap")]
            BackendOutgoing::TunUnbind { slot } => {
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
            BackendOutgoing::TunPacket { slot, data } => {
                if let Some(Some(SocketType::Tun(fd, _))) = self.sockets.get_mut(slot) {
                    if let Err(e) = fd.fd.write_all(&data) {
                        log::error!("Poll write_all error {:?}", e);
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
        backend::{Backend, BackendOwner},
        group_owner_type,
    };

    use super::PollingBackend;

    group_owner_type!(SimpleOwner);

    #[allow(unused_assignments)]
    #[cfg(feature = "udp")]
    #[test]
    fn test_on_action_udp_listen_success() {
        use std::ops::Deref;

        use crate::{
            backend::{BackendIncoming, BackendIncomingInternal, BackendOutgoing},
            Buffer,
        };

        let mut backend = PollingBackend::<SimpleOwner, 2, 2>::default();

        let mut addr1 = None;
        let mut slot1 = 0;
        let mut addr2 = None;
        let mut slot2 = 0;

        backend.on_action(
            SimpleOwner(1),
            BackendOutgoing::UdpListen {
                addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                reuse: false,
            },
        );
        backend.poll_incoming(Duration::from_millis(100));
        match backend.pop_incoming() {
            Some(BackendIncomingInternal::Event(
                owner,
                BackendIncoming::UdpListenResult { bind, result },
            )) => {
                assert_eq!(owner, SimpleOwner(1));
                assert_eq!(bind, SocketAddr::from(([127, 0, 0, 1], 0)));
                let res = result.expect("Expected Ok");
                addr1 = Some(res.0);
                slot1 = res.1;
            }
            _ => panic!("Expected UdpListenResult"),
        }

        backend.on_action(
            SimpleOwner(2),
            BackendOutgoing::UdpListen {
                addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                reuse: false,
            },
        );
        backend.poll_incoming(Duration::from_millis(100));
        match backend.pop_incoming() {
            Some(BackendIncomingInternal::Event(
                owner,
                BackendIncoming::UdpListenResult { bind, result },
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
            BackendOutgoing::UdpPacket {
                slot: slot1,
                to: addr2.expect(""),
                data: Buffer::from(b"hello".to_vec()),
            },
        );

        backend.poll_incoming(Duration::from_millis(100));
        for _ in 0..10 {
            match backend.pop_incoming() {
                Some(BackendIncomingInternal::Event(
                    owner,
                    BackendIncoming::UdpPacket { from, slot, data },
                )) => {
                    assert_eq!(owner, SimpleOwner(2));
                    assert_eq!(from, addr1.expect(""));
                    assert_eq!(slot, slot2);
                    assert_eq!(data.deref(), b"hello");
                    break;
                }
                _ => {}
            }
        }

        backend.remove_owner(SimpleOwner(1));
        assert_eq!(backend.socket_count(), 2);

        backend.remove_owner(SimpleOwner(2));
        assert_eq!(backend.socket_count(), 1);
    }
}
