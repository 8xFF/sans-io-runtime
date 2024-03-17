#[cfg(feature = "udp")]
use std::net::SocketAddr;
use std::{fmt::Debug, sync::Arc, time::Duration};

use crate::{owner::Owner, task::NetOutgoing};

#[cfg(feature = "mio-backend")]
mod mio;

#[cfg(feature = "poll-backend")]
mod poll;

#[cfg(feature = "polling-backend")]
mod polling;

#[cfg(feature = "mio-backend")]
pub use mio::MioBackend;

#[cfg(feature = "poll-backend")]
pub use poll::PollBackend;

#[cfg(feature = "polling-backend")]
pub use polling::PollingBackend;

/// Represents an incoming network event.
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
        len: usize,
    },
    #[cfg(feature = "tun-tap")]
    TunBindResult {
        result: Result<usize, std::io::Error>,
    },
    #[cfg(feature = "tun-tap")]
    TunPacket {
        slot: usize,
        len: usize,
    },
    Awake,
}

pub trait Awaker: Send + Sync {
    fn awake(&self);
}

pub trait Backend: Default + BackendOwner {
    fn create_awaker(&self) -> Arc<dyn Awaker>;
    fn poll_incoming(&mut self, timeout: Duration);
    fn pop_incoming(&mut self, buf: &mut [u8]) -> Option<(BackendIncoming, Owner)>;
    fn finish_outgoing_cycle(&mut self);
    fn finish_incoming_cycle(&mut self);
}

pub trait BackendOwner {
    fn on_action(&mut self, owner: Owner, action: NetOutgoing);
    fn remove_owner(&mut self, owner: Owner);
}

#[cfg(feature = "tun-tap")]
pub mod tun {
    use std::{net::Ipv4Addr, process::Command};

    use tun::{
        platform::{posix::Fd, Device},
        Device as _, IntoAddress,
    };

    pub struct TunFd {
        pub fd: Fd,
        pub read: bool,
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

    pub fn create_tun<A: IntoAddress>(name: &str, ip: A, netmask: A, queues: usize) -> TunDevice {
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
            .mtu(1180)
            .up();

        #[cfg(target_os = "linux")]
        config.queues(queues);

        let device = tun::create(&config).expect("Should create tun device");
        device
            .set_nonblock()
            .expect("Should set vpn tun device nonblock");

        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            let ip_addr = device.address().expect("Should have address");
            //let netmask = device.netmask().expect("Should have netmask");

            //TODO avoid using fixed value
            let output = Command::new("route")
                .args(&[
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
