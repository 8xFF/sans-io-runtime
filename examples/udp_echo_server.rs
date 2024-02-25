use std::{collections::VecDeque, net::SocketAddr, time::Duration};

use sans_io_runtime::{
    Controller, MioBackend, NetIncoming, NetOutgoing, WorkerCtx, WorkerInner, WorkerInnerOutput,
    WorkerStats,
};

type ExtIn = ();
type ExtOut = ();
type Owner = ();
type SCfg = ();

struct EchoWorkerCfg {
    bind: SocketAddr,
}

enum EchoWorkerInQueue {
    UdpListen(SocketAddr),
    SendUdpPacket {
        from: SocketAddr,
        to: SocketAddr,
        buf_index: usize,
        len: usize,
    },
}

struct EchoWorker {
    buffers: [[u8; 1500]; 512],
    buffer_index: usize,
    output: VecDeque<EchoWorkerInQueue>,
}

impl EchoWorker {
    pub fn new(cfg: EchoWorkerCfg) -> Self {
        log::info!("Create new echo task in addr {}", cfg.bind);
        Self {
            buffers: [[0; 1500]; 512],
            buffer_index: 0,
            output: VecDeque::from([EchoWorkerInQueue::UdpListen(cfg.bind)]),
        }
    }
}

impl WorkerInner<ExtIn, ExtOut, EchoWorkerCfg, (), ()> for EchoWorker {
    fn build(cfg: EchoWorkerCfg) -> Self {
        Self::new(cfg)
    }

    fn tasks(&self) -> usize {
        1
    }

    fn spawn(&mut self, ctx: WorkerCtx<'_, Owner>, cfg: SCfg) {}
    fn on_ext(&mut self, ext: ExtIn) {}
    fn on_net(&mut self, owner: Owner, input: NetIncoming) {
        match input {
            NetIncoming::UdpListenResult { bind, result } => {
                log::info!("UdpListenResult: {} {:?}", bind, result);
            }
            NetIncoming::UdpPacket { from, to, data } => {
                assert!(data.len() <= 1500, "data too large");
                let buffer_index = self.buffer_index;
                self.buffer_index = (self.buffer_index + 1) % self.buffers.len();
                self.buffers[buffer_index][0..data.len()].copy_from_slice(data);

                self.output.push_back(EchoWorkerInQueue::SendUdpPacket {
                    from: to,
                    to: from,
                    buf_index: buffer_index,
                    len: data.len(),
                });
            }
            _ => unreachable!("EchoWorker only has NetIncoming variants"),
        }
    }
    fn inner_process(&mut self) {}
    fn pop_output(&mut self) -> Option<WorkerInnerOutput<'_, ExtOut, Owner>> {
        let out = self.output.pop_front()?;
        match out {
            EchoWorkerInQueue::UdpListen(bind) => {
                Some(WorkerInnerOutput::Net(NetOutgoing::UdpListen(bind), ()))
            }
            EchoWorkerInQueue::SendUdpPacket {
                from,
                to,
                buf_index,
                len,
            } => Some(WorkerInnerOutput::Net(
                NetOutgoing::UdpPacket {
                    from,
                    to,
                    data: &self.buffers[buf_index][0..len],
                },
                (),
            )),
        }
    }
}

fn main() {
    env_logger::init();
    let mut controller = Controller::<ExtIn, ExtOut, SCfg>::new(2);
    controller.add_worker::<(), _, EchoWorker, MioBackend<()>>(EchoWorkerCfg {
        bind: SocketAddr::from(([127, 0, 0, 1], 10001)),
    });
    loop {
        controller.process();
        std::thread::sleep(Duration::from_millis(100));
    }
}
