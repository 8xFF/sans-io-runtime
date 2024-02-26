use std::{
    collections::VecDeque,
    net::SocketAddr,
    time::{Duration, Instant},
};

use sans_io_runtime::{
    Controller, MioBackend, NetIncoming, NetOutgoing, Owner, WorkerCtx, WorkerInner,
    WorkerInnerOutput,
};

type ExtIn = ();
type ExtOut = ();
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
    worker: u16,
    buffers: [[u8; 1500]; 512],
    buffer_index: usize,
    output: VecDeque<EchoWorkerInQueue>,
}

impl WorkerInner<ExtIn, ExtOut, EchoWorkerCfg, ()> for EchoWorker {
    fn build(worker: u16, cfg: EchoWorkerCfg) -> Self {
        log::info!("Create new echo task in addr {}", cfg.bind);
        Self {
            worker,
            buffers: [[0; 1500]; 512],
            buffer_index: 0,
            output: VecDeque::from([EchoWorkerInQueue::UdpListen(cfg.bind)]),
        }
    }

    fn tasks(&self) -> usize {
        1
    }

    fn spawn(&mut self, _now: Instant, _ctx: &mut WorkerCtx<'_>, _cfg: SCfg) {}
    fn on_ext(&mut self, _now: Instant, _ctx: &mut WorkerCtx<'_>, _ext: ExtIn) {}
    fn on_net(&mut self, _now: Instant, _owner: Owner, input: NetIncoming) {
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
        }
    }
    fn inner_process(&mut self, _now: Instant, _ctx: &mut WorkerCtx<'_>) {}
    fn pop_output(&mut self) -> Option<WorkerInnerOutput<'_, ExtOut>> {
        let out = self.output.pop_front()?;
        match out {
            EchoWorkerInQueue::UdpListen(bind) => Some(WorkerInnerOutput::Net(
                NetOutgoing::UdpListen(bind),
                Owner::worker(self.worker),
            )),
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
                Owner::worker(self.worker),
            )),
        }
    }
}

fn main() {
    env_logger::init();
    let mut controller = Controller::<ExtIn, ExtOut, SCfg>::new();
    controller.add_worker::<_, EchoWorker, MioBackend>(EchoWorkerCfg {
        bind: SocketAddr::from(([127, 0, 0, 1], 10001)),
    });
    controller.add_worker::<_, EchoWorker, MioBackend>(EchoWorkerCfg {
        bind: SocketAddr::from(([127, 0, 0, 1], 10002)),
    });
    loop {
        controller.process();
        std::thread::sleep(Duration::from_millis(100));
    }
}
