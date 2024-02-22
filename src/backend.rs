pub enum SansIoBackendEvent<'a> {
    UdpListen(std::net::SocketAddr),
    UdpPacket {
        from: std::net::SocketAddr,
        to: std::net::SocketAddr,
        data: &'a [u8],
    },
}

pub trait SansIoBackend: Default {}
