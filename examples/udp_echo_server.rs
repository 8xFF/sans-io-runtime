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
    UdpListen(SocketAddr),
    SendUdpPacket {
        from: SocketAddr,
        to: SocketAddr,
        buf_index: usize,
        len: usize,
    },
    Destroy,
}

struct EchoTask {
    buffers: [[u8; 1500]; 128],
    buffer_index: usize,
    output: VecDeque<EchoTaskInQueue>,
}

impl EchoTask {
    pub fn new(cfg: EchoTaskCfg) -> Self {
        log::info!("Create new echo task in addr {}", cfg.bind);
        Self {
            buffers: [[0; 1500]; 128],
            buffer_index: 0,
            output: VecDeque::from([EchoTaskInQueue::UdpListen(cfg.bind)]),
        }
    }
}

impl Task<ExtIn, ExtOut, MSG, EchoTaskCfg> for EchoTask {
    fn build(cfg: EchoTaskCfg) -> Self {
        Self::new(cfg)
    }

    fn min_tick_interval(&self) -> Duration {
        Duration::from_millis(1)
    }

    fn on_tick(&mut self, _now: Instant) {}

    fn on_input<'b>(&mut self, _now: Instant, input: Input<'b, ExtIn, MSG>) {
        match input {
            Input::Net(NetIncoming::UdpListenResult { bind, result }) => {
                log::info!("UdpListenResult: {} {:?}", bind, result);
            }
            Input::Net(NetIncoming::UdpPacket { from, to, data }) => {
                assert!(data.len() <= 1500, "data too large");
                log::info!("UdpPacket: {} -> {} {:?}", from, to, data);
                let buffer_index = self.buffer_index;
                self.buffer_index = (self.buffer_index + 1) % self.buffers.len();
                self.buffers[buffer_index][0..data.len()].copy_from_slice(data);

                self.output.push_back(EchoTaskInQueue::SendUdpPacket {
                    from: to,
                    to: from,
                    buf_index: buffer_index,
                    len: data.len(),
                });

                if data == b"quit\n" {
                    log::info!("Destroying task");
                    self.output.push_back(EchoTaskInQueue::Destroy);
                } else {
                    log::info!("Echoing data not same with quit {:?}", b"quit\n")
                }
            }
            _ => unreachable!("EchoTask only has NetIncoming variants"),
        }
    }

    fn pop_output(&mut self, _now: Instant) -> Option<Output<'_, ExtOut, MSG>> {
        let out = self.output.pop_front()?;
        match out {
            EchoTaskInQueue::UdpListen(bind) => Some(Output::Net(NetOutgoing::UdpListen(bind))),
            EchoTaskInQueue::SendUdpPacket {
                from,
                to,
                buf_index,
                len,
            } => Some(Output::Net(NetOutgoing::UdpPacket {
                from,
                to,
                data: &self.buffers[buf_index][0..len],
            })),
            EchoTaskInQueue::Destroy => Some(Output::Destroy),
        }
    }
}

fn main() {
    env_logger::init();
    let mut controller =
        Controller::<ExtIn, ExtOut, MSG, EchoTask, EchoTaskCfg, MioBackend<usize>>::new(1);
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
