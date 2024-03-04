use std::{collections::VecDeque, net::SocketAddr, time::Duration};

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
    TcpListener(SocketAddr),
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
            output: VecDeque::from([EchoWorkerInQueue::TcpListener(cfg.bind)]),
        }
    }

    fn worker_index(&self) -> u16 {
        self.worker
    }

    fn tasks(&self) -> usize {
        1
    }

    fn spawn(&mut self, _now: std::time::Instant, _cfg: SCfg) {}

    fn on_input_tick<'a>(
        &mut self,
        _now: std::time::Instant,
    ) -> Option<sans_io_runtime::WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        match self.output.pop_front()? {
            EchoWorkerInQueue::TcpListener(bind) => Some(WorkerInnerOutput::Task(
                Owner::worker(self.worker),
                TaskOutput::Net(NetOutgoing::TcpListen(bind)),
            )),
        }
    }

    fn on_input_event<'a>(
        &mut self,
        _now: std::time::Instant,
        event: sans_io_runtime::WorkerInnerInput<'a, ExtIn, ChannelId, Event>,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        match event {
            WorkerInnerInput::Task(
                _owner,
                TaskInput::Net(NetIncoming::TcpListenResult { bind, result }),
            ) => {
                log::info!("TcpListen Result: {} {:?}", bind, result);
                None
            }
            WorkerInnerInput::Task(
                _owner,
                TaskInput::Net(NetIncoming::TcpOnConnected {
                    local_addr,
                    remote_addr,
                }),
            ) => {
                log::info!("Client {} is connected", remote_addr);
                None
            }
            WorkerInnerInput::Task(
                _owner,
                TaskInput::Net(NetIncoming::TcpOnDisconnected {
                    local_addr,
                    remote_addr,
                }),
            ) => {
                log::info!("Client {} is disconnected", remote_addr);
                None
            }
            WorkerInnerInput::Task(
                _owner,
                TaskInput::Net(NetIncoming::TcpPacket { from, to, data }),
            ) => Some(WorkerInnerOutput::Task(
                Owner::worker(self.worker),
                TaskOutput::Net(NetOutgoing::TcpPacket {
                    from: to,
                    to: from,
                    data: Buffer::Vec(data.to_vec()),
                }),
            )),
            _ => None,
        }
    }

    fn pop_last_input<'a>(
        &mut self,
        _now: std::time::Instant,
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
    loop {
        controller.process();
        std::thread::sleep(Duration::from_millis(100));
    }
}
