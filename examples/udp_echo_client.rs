use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use sans_io_runtime::{
    backend::{BackendIncoming, BackendOutgoing, PollBackend},
    group_owner_type, Buffer, Controller, ErrorDebugger2, Task, TaskGroup, WorkerInner,
    WorkerInnerInput, WorkerInnerOutput,
};

type ExtIn = ();
type ExtOut = ();
type ICfg = ();
type SCfg = EchoTaskCfg;
type ChannelId = ();
type Event = ();

enum EchoTaskOutput {
    Net(BackendOutgoing),
    Destroy,
}

#[derive(Debug, Clone)]
struct EchoTaskCfg {
    count: usize,
    dest: SocketAddr,
    brust_size: usize,
}

struct EchoTask {
    count: usize,
    cfg: EchoTaskCfg,
    local_addr: SocketAddr,
    local_backend_slot: usize,
    output: heapless::Deque<EchoTaskOutput, 16>,
}

impl EchoTask {
    pub fn new(cfg: EchoTaskCfg) -> Self {
        log::info!("Create new echo client task in addr {}", cfg.dest);
        let mut output = heapless::Deque::new();
        output
            .push_back(EchoTaskOutput::Net(BackendOutgoing::UdpListen {
                addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                reuse: false,
            }))
            .print_err2("should not hapended");
        Self {
            count: 0,
            cfg,
            local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            local_backend_slot: 0,
            output,
        }
    }
}

impl Task<BackendIncoming, EchoTaskOutput> for EchoTask {
    fn on_tick(&mut self, _now: Instant) -> Option<EchoTaskOutput> {
        self.output.pop_front()
    }

    fn on_event(&mut self, _now: Instant, input: BackendIncoming) -> Option<EchoTaskOutput> {
        match input {
            BackendIncoming::UdpListenResult { bind, result } => {
                log::info!("UdpListenResult: {} {:?}", bind, result);
                if let Ok((addr, slot)) = result {
                    self.local_addr = addr;
                    self.local_backend_slot = slot;
                    for _ in 0..self.cfg.brust_size {
                        self.output
                            .push_back(EchoTaskOutput::Net(BackendOutgoing::UdpPacket {
                                slot: self.local_backend_slot,
                                to: self.cfg.dest,
                                data: Buffer::from([0; 1000].to_vec()),
                            }))
                            .print_err2("Should push ok");
                    }
                }
                self.output.pop_front()
            }
            BackendIncoming::UdpPacket { from, slot, data } => {
                self.count += 1;
                if self.count >= self.cfg.count {
                    log::info!("EchoTask done");
                    Some(EchoTaskOutput::Destroy)
                } else {
                    Some(EchoTaskOutput::Net(BackendOutgoing::UdpPacket {
                        slot,
                        to: from,
                        data,
                    }))
                }
            }
            _ => None,
        }
    }

    fn pop_output(&mut self, _now: Instant) -> Option<EchoTaskOutput> {
        self.output.pop_front()
    }

    fn shutdown(&mut self, _now: Instant) -> Option<EchoTaskOutput> {
        log::info!("EchoTask shutdown");
        self.output
            .push_back(EchoTaskOutput::Net(BackendOutgoing::UdpUnlisten {
                slot: self.local_backend_slot,
            }))
            .print_err2("should not hapended");
        self.output
            .push_back(EchoTaskOutput::Destroy)
            .print_err2("should not hapended");
        self.output.pop_front()
    }
}

group_owner_type!(OwnerType);

struct EchoWorkerInner {
    worker: u16,
    tasks: TaskGroup<BackendIncoming, EchoTaskOutput, EchoTask, 16>,
}

impl WorkerInner<OwnerType, ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg> for EchoWorkerInner {
    fn tasks(&self) -> usize {
        self.tasks.tasks()
    }

    fn worker_index(&self) -> u16 {
        self.worker
    }

    fn build(worker: u16, _cfg: ICfg) -> Self {
        Self {
            worker,
            tasks: TaskGroup::default(),
        }
    }

    fn spawn(&mut self, _now: Instant, cfg: SCfg) {
        self.tasks.add_task(EchoTask::new(cfg));
    }

    fn on_tick(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<OwnerType, ExtOut, ChannelId, Event, SCfg>> {
        let (index, out) = self.tasks.on_tick(now)?;
        self.convert_output(OwnerType(index), out)
    }

    fn on_event(
        &mut self,
        now: Instant,
        event: WorkerInnerInput<OwnerType, ExtIn, ChannelId, Event>,
    ) -> Option<WorkerInnerOutput<OwnerType, ExtOut, ChannelId, Event, SCfg>> {
        match event {
            WorkerInnerInput::Net(owner, event) => {
                let out = self.tasks.on_event(now, owner.index(), event)?;
                self.convert_output(owner, out)
            }
            _ => unreachable!(),
        }
    }

    fn pop_output(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<OwnerType, ExtOut, ChannelId, Event, SCfg>> {
        let (index, out) = self.tasks.pop_output(now)?;
        self.convert_output(OwnerType(index), out)
    }

    fn shutdown(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<OwnerType, ExtOut, ChannelId, Event, SCfg>> {
        let (index, out) = self.tasks.shutdown(now)?;
        self.convert_output(OwnerType(index), out)
    }
}

impl EchoWorkerInner {
    fn convert_output(
        &mut self,
        owner: OwnerType,
        out: EchoTaskOutput,
    ) -> Option<WorkerInnerOutput<OwnerType, ExtOut, ChannelId, Event, SCfg>> {
        match out {
            EchoTaskOutput::Net(out) => Some(WorkerInnerOutput::Net(owner, out)),
            EchoTaskOutput::Destroy => {
                self.tasks.remove_task(owner.index());
                Some(WorkerInnerOutput::Destroy(owner))
            }
        }
    }
}

fn main() {
    env_logger::init();
    println!("{}", std::mem::size_of::<EchoWorkerInner>());
    let mut controller =
        Controller::<ExtIn, ExtOut, EchoTaskCfg, ChannelId, Event, 1024>::default();
    controller.add_worker::<OwnerType, _, EchoWorkerInner, PollBackend<_, 1024, 1024>>(
        Duration::from_secs(1),
        (),
        None,
    );
    controller.add_worker::<OwnerType, _, EchoWorkerInner, PollBackend<_, 1024, 1024>>(
        Duration::from_secs(1),
        (),
        None,
    );

    for _i in 0..2 {
        controller.spawn(EchoTaskCfg {
            count: 1000,
            brust_size: 1,
            dest: SocketAddr::from(([127, 0, 0, 1], 10001)),
        });
        controller.spawn(EchoTaskCfg {
            count: 10000,
            brust_size: 1,
            dest: SocketAddr::from(([127, 0, 0, 1], 10002)),
        });
    }

    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term))
        .expect("Should register hook");

    while controller.process().is_some() {
        if term.load(Ordering::Relaxed) {
            controller.shutdown();
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    log::info!("Server shutdown");
}
