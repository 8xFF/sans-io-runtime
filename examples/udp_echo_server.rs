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
type ChannelId = ();
type Event = ();
type ICfg = EchoWorkerCfg;
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

impl WorkerInner<ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg> for EchoWorker {
    fn build(worker: u16, cfg: EchoWorkerCfg) -> Self {
        log::info!("Create new echo task in addr {}", cfg.bind);
        Self {
            worker,
            buffers: [[0; 1500]; 512],
            buffer_index: 0,
            output: VecDeque::from([EchoWorkerInQueue::UdpListen(cfg.bind)]),
        }
    }

    fn worker_index(&self) -> u16 {
        self.worker
    }

    fn tasks(&self) -> usize {
        1
    }

    fn spawn(&mut self, _now: Instant, _ctx: &mut WorkerCtx<'_>, _cfg: SCfg) {}
    fn on_ext(&mut self, _now: Instant, _ctx: &mut WorkerCtx<'_>, _ext: ExtIn) {}
    fn on_bus(
        &mut self,
        _now: Instant,
        _ctx: &mut WorkerCtx<'_>,
        _owner: Owner,
        _channel_id: ChannelId,
        _event: Event,
    ) {
    }
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
    fn pop_output(&mut self) -> Option<WorkerInnerOutput<'_, ExtOut, ChannelId, Event>> {
        let out = self.output.pop_front()?;
        match out {
            EchoWorkerInQueue::UdpListen(bind) => Some(WorkerInnerOutput::Net(
                Owner::worker(self.worker),
                NetOutgoing::UdpListen(bind),
            )),
            EchoWorkerInQueue::SendUdpPacket {
                from,
                to,
                buf_index,
                len,
            } => Some(WorkerInnerOutput::Net(
                Owner::worker(self.worker),
                NetOutgoing::UdpPacket {
                    from,
                    to,
                    data: &self.buffers[buf_index][0..len],
                },
            )),
        }
    }
}

fn main() {
    env_logger::init();
    let mut controller = Controller::<ExtIn, ExtOut, SCfg, ChannelId, Event, 1024>::new();
    controller.add_worker::<_, EchoWorker, MioBackend<16, 1024>>(
        EchoWorkerCfg {
            bind: SocketAddr::from(([127, 0, 0, 1], 10001)),
        },
        None,
    );
    controller.add_worker::<_, EchoWorker, MioBackend<16, 1024>>(
        EchoWorkerCfg {
            bind: SocketAddr::from(([127, 0, 0, 1], 10002)),
        },
        None,
    );
    loop {
        controller.process();
        std::thread::sleep(Duration::from_millis(100));
    }
}
