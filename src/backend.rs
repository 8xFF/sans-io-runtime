#[cfg(feature = "udp")]
use std::net::SocketAddr;
use std::{fmt::Debug, sync::Arc, time::Duration};

#[cfg(feature = "poll-backend")]
mod poll;

#[cfg(feature = "polling-backend")]
mod polling;

#[cfg(feature = "poll-backend")]
pub use poll::PollBackend;

#[cfg(feature = "polling-backend")]
pub use polling::PollingBackend;

use crate::Buffer;

#[cfg(feature = "tun-tap")]
use self::tun::TunFd;

#[derive(Debug)]
pub enum BackendIncoming {
    #[cfg(feature = "udp")]
    UdpListenResult {
        bind: SocketAddr,
        result: Result<(SocketAddr, usize), std::io::Error>,
    },
    #[cfg(feature = "udp")]
    UdpPacket {
        slot: usize,
        from: SocketAddr,
        data: Buffer,
    },
    #[cfg(feature = "tun-tap")]
    TunBindResult {
        result: Result<usize, std::io::Error>,
    },
    #[cfg(feature = "tun-tap")]
    TunPacket { slot: usize, data: Buffer },
}

/// Represents an incoming network event.
#[derive(Debug)]
pub enum BackendIncomingInternal<Owner> {
    Event(Owner, BackendIncoming),
    Awake,
}

/// Represents an outgoing network control.
#[derive(Debug, PartialEq, Eq)]
pub enum BackendOutgoing {
    #[cfg(feature = "udp")]
    UdpListen { addr: SocketAddr, reuse: bool },
    #[cfg(feature = "udp")]
    UdpUnlisten { slot: usize },
    #[cfg(feature = "udp")]
    UdpPacket {
        slot: usize,
        to: SocketAddr,
        data: Buffer,
    },
    #[cfg(feature = "udp")]
    UdpPackets {
        slot: usize,
        to: Vec<SocketAddr>,
        data: Buffer,
    },
    #[cfg(feature = "udp")]
    UdpPackets2 {
        to: Vec<(usize, SocketAddr)>,
        data: Buffer,
    },
    #[cfg(feature = "tun-tap")]
    TunBind { fd: TunFd },
    #[cfg(feature = "tun-tap")]
    TunUnbind { slot: usize },
    #[cfg(feature = "tun-tap")]
    TunPacket { slot: usize, data: Buffer },
}

pub trait Awaker: Send + Sync {
    fn awake(&self);
}

pub trait Backend<Owner>: Default + BackendOwner<Owner> {
    fn create_awaker(&self) -> Arc<dyn Awaker>;
    fn poll_incoming(&mut self, timeout: Duration);
    fn pop_incoming(&mut self) -> Option<BackendIncomingInternal<Owner>>;
    fn finish_outgoing_cycle(&mut self);
    fn finish_incoming_cycle(&mut self);
}

pub trait BackendOwner<Owner> {
    fn on_action(&mut self, owner: Owner, action: BackendOutgoing);
    fn remove_owner(&mut self, owner: Owner);
}

#[cfg(feature = "tun-tap")]
pub mod tun {
    use std::net::Ipv4Addr;

    use tun::{
        platform::{posix::Fd, Device},
        IntoAddress,
    };

    pub struct TunFd {
        pub fd: Fd,
        pub read: bool,
    }

    impl Eq for TunFd {}

    impl PartialEq for TunFd {
        fn eq(&self, other: &Self) -> bool {
            self.fd.0 == other.fd.0
        }
    }

    impl std::fmt::Debug for TunFd {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "TunFd({})", self.fd.0)
        }
    }

    pub struct TunDevice {
        device: Device,
    }

    impl TunDevice {
        pub fn get_queue_fd(&mut self, index: usize) -> Option<TunFd> {
            use std::os::fd::AsRawFd;
            use tun::Device;

            #[cfg(target_os = "macos")]
            let queue = self.device.queue(0)?;
            #[cfg(target_os = "linux")]
            let queue = self.device.queue(index)?;
            queue.set_nonblock().expect("Should set tun nonblock");
            let raw_fd = queue.as_raw_fd();
            Some(TunFd {
                fd: Fd(raw_fd),
                #[cfg(target_os = "macos")]
                read: index == 0,
                #[cfg(target_os = "linux")]
                read: true,
            })
        }
    }

    impl Drop for TunDevice {
        fn drop(&mut self) {
            log::info!("Dropping tun device");
        }
    }

    pub fn create_tun<A: IntoAddress>(
        name: &str,
        ip: A,
        netmask: A,
        mtu: u16,
        queues: usize,
    ) -> TunDevice {
        let mut config = tun::Configuration::default();
        let ip: Ipv4Addr = ip.into_address().expect("Should convert to ip-v4");
        let netmask: Ipv4Addr = netmask.into_address().expect("Should convert to ip-v4");
        log::info!(
            "Creating tun device with ip: {} and netmask: {}",
            ip,
            netmask
        );
        config
            .name(name)
            .address(ip)
            .destination(ip)
            .netmask(netmask)
            .mtu(mtu as i32)
            .up();

        #[cfg(target_os = "linux")]
        config.queues(queues);
        #[cfg(not(target_os = "linux"))]
        log::info!("Ignoring queues on non-linux platform, setting as {queues} but override to 1");

        let device = tun::create(&config).expect("Should create tun device");
        device
            .set_nonblock()
            .expect("Should set vpn tun device nonblock");

        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            use std::process::Command;
            use tun::Device as _;

            let ip_addr = device.address().expect("Should have address");
            //let netmask = device.netmask().expect("Should have netmask");

            //TODO avoid using fixed value
            let output = Command::new("route")
                .args([
                    "-n",
                    "add",
                    "-net",
                    "10.33.33.0/24",
                    &format!("{}", ip_addr),
                ])
                .output();
            match output {
                Ok(output) => {
                    if !output.status.success() {
                        log::error!(
                            "Add route to tun device error {}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                    } else {
                        log::info!("Add route to tun device success");
                    }
                }
                Err(e) => {
                    log::error!("Add route to tune device error {}", e);
                }
            }
        }

        TunDevice { device }
    }
}
