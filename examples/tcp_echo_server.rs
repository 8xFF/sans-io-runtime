use std::{
    collections::VecDeque,
    net::SocketAddr,
    time::{Duration, Instant},
};

use sans_io_runtime::{Controller, Input, MioBackend, NetIncoming, NetOutgoing, Output, Task};

type ExtIn = ();
type ExtOut = ();
type MSG = ();
struct EchoTaskCfg {
    bind: SocketAddr,
}

enum EchoTaskInQueue {
    TcpListen(SocketAddr),
    SendTcpPacket {
        from: SocketAddr,
        to: SocketAddr,
        buf_index: usize,
        len: usize,
    },
    Close {
        remote_addr: SocketAddr,
    },
    Destroy,
}

struct EchoTask {
    addr: SocketAddr,
    buffers: [[u8; 1500]; 512],
    buffer_index: usize,
    output: VecDeque<EchoTaskInQueue>,
}

impl EchoTask {
    pub fn new(cfg: EchoTaskCfg) -> Self {
        log::info!("Create new echo task in addr {}", cfg.bind);
        Self {
            addr: cfg.bind,
            buffers: [[0; 1500]; 512],
            buffer_index: 0,
            output: VecDeque::from([EchoTaskInQueue::TcpListen(cfg.bind)]),
        }
    }
}

impl Task<ExtIn, ExtOut, MSG, EchoTaskCfg> for EchoTask {
    fn build(cfg: EchoTaskCfg) -> Self {
        Self::new(cfg)
    }

    fn min_tick_interval(&self) -> std::time::Duration {
        Duration::from_millis(1)
    }

    fn on_tick(&mut self, now: Instant) {}

    fn on_input<'a>(&mut self, now: Instant, input: sans_io_runtime::Input<'a, ExtIn, MSG>) {
        match input {
            Input::Net(NetIncoming::TcpListenResult { bind, result }) => {
                log::info!("TcpListenResult: {} {:?}", bind, result);
            }
            Input::Net(NetIncoming::TcpNewConnection { remote_addr }) => {
                log::info!("Tcp new connection: {}", remote_addr);
            }
            Input::Net(NetIncoming::TcpConnectionClose { addr }) => {
                log::info!("Tcp connection closed {}", addr);
            }
            Input::Net(NetIncoming::TcpPacket { from, to, data }) => {
                assert!(data.len() <= 1500, "data too large");
                let buffer_index = self.buffer_index;
                self.buffer_index = (self.buffer_index + 1) % self.buffers.len();
                self.buffers[buffer_index][0..data.len()].copy_from_slice(data);

                self.output.push_back(EchoTaskInQueue::SendTcpPacket {
                    from: to,
                    to: from,
                    buf_index: buffer_index,
                    len: data.len(),
                });

                // println!("got a packet: {:?}", data);

                if data == b"close\r\n" {
                    log::info!("close current connection");
                    self.output
                        .push_back(EchoTaskInQueue::Close { remote_addr: from });
                }

                if data == b"exit\r\n" {
                    log::info!("destroy current task");
                    self.output.push_back(EchoTaskInQueue::Destroy);
                }
            }
            _ => unreachable!("EchoTask EchoTask only has NetIncoming variants"),
        }
    }

    fn pop_output(&mut self, now: Instant) -> Option<sans_io_runtime::Output<'_, ExtOut, MSG>> {
        match self.output.pop_front() {
            Some(out) => match out {
                EchoTaskInQueue::TcpListen(bind) => Some(Output::Net(NetOutgoing::TcpListen(bind))),
                EchoTaskInQueue::SendTcpPacket {
                    from,
                    to,
                    buf_index,
                    len,
                } => Some(Output::Net(NetOutgoing::TcpPacket {
                    from,
                    to,
                    data: &self.buffers[buf_index][0..len],
                })),
                EchoTaskInQueue::Close { remote_addr } => {
                    Some(Output::Net(NetOutgoing::TcpClose {
                        local_addr: self.addr,
                        remote_addr,
                    }))
                }
                EchoTaskInQueue::Destroy => Some(Output::Destroy),
            },
            None => None,
        }
    }
}

fn main() {
    env_logger::init();
    let mut controller =
        Controller::<ExtIn, ExtOut, MSG, EchoTask, EchoTaskCfg, MioBackend<usize>>::new(2);
    controller.start();
    controller.spawn(EchoTaskCfg {
        bind: SocketAddr::from(([127, 0, 0, 1], 10001)),
    });
    controller.spawn(EchoTaskCfg {
        bind: SocketAddr::from(([127, 0, 0, 1], 10002)),
    });
    loop {
        controller.process();
        std::thread::sleep(Duration::from_millis(100));
    }
}
