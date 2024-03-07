use std::{
    collections::VecDeque,
    net::SocketAddr,
    time::{Duration, Instant},
};

use sans_io_runtime::{
    backend::MioBackend, Buffer, Controller, NetIncoming, NetOutgoing, Owner, TaskInput,
    TaskOutput, WorkerInner, WorkerInnerInput, WorkerInnerOutput,
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
}

struct EchoWorker {
    worker: u16,
    output: VecDeque<EchoWorkerInQueue>,
}

impl WorkerInner<ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg> for EchoWorker {
    fn build(worker: u16, cfg: EchoWorkerCfg) -> Self {
        log::info!("Create new echo task in addr {}", cfg.bind);
        Self {
            worker,
            output: VecDeque::from([EchoWorkerInQueue::UdpListen(cfg.bind)]),
        }
    }

    fn worker_index(&self) -> u16 {
        self.worker
    }

    fn tasks(&self) -> usize {
        1
    }

    fn spawn(&mut self, _now: Instant, _cfg: SCfg) {}
    fn on_tick<'a>(
        &mut self,
        _now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        match self.output.pop_front()? {
            EchoWorkerInQueue::UdpListen(addr) => Some(WorkerInnerOutput::Task(
                Owner::worker(self.worker),
                TaskOutput::Net(NetOutgoing::UdpListen { addr, reuse: false }),
            )),
        }
    }
    fn on_event<'a>(
        &mut self,
        _now: Instant,
        event: WorkerInnerInput<'a, ExtIn, ChannelId, Event>,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        match event {
            WorkerInnerInput::Task(
                _owner,
                TaskInput::Net(NetIncoming::UdpListenResult { bind, result }),
            ) => {
                log::info!("UdpListenResult: {} {:?}", bind, result);
                None
            }
            WorkerInnerInput::Task(
                _owner,
                TaskInput::Net(NetIncoming::UdpPacket { from, slot, data }),
            ) => {
                assert!(data.len() <= 1500, "data too large");

                Some(WorkerInnerOutput::Task(
                    Owner::worker(self.worker),
                    TaskOutput::Net(NetOutgoing::UdpPacket {
                        slot,
                        to: from,
                        data: Buffer::Vec(data.to_vec()),
                    }),
                ))
            }
            _ => None,
        }
    }

    fn pop_output<'a>(
        &mut self,
        _now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        None
    }
}

fn main() {
    env_logger::init();
    let mut controller = Controller::<ExtIn, ExtOut, SCfg, ChannelId, Event, 1024>::default();
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
