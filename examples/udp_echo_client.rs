use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use sans_io_runtime::{
    backend::PollBackend, Buffer, Controller, ErrorDebugger2, NetIncoming, NetOutgoing, Task,
    TaskGroup, TaskGroupInput, TaskInput, TaskOutput, WorkerInner, WorkerInnerInput,
    WorkerInnerOutput,
};

type ExtIn = ();
type ExtOut = ();
type ICfg = ();
type SCfg = EchoTaskCfg;
type ChannelId = ();
type Event = ();

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
    output: heapless::Deque<TaskOutput<'static, ExtOut, ChannelId, ChannelId, Event>, 16>,
}

impl EchoTask {
    pub fn new(cfg: EchoTaskCfg) -> Self {
        log::info!("Create new echo client task in addr {}", cfg.dest);
        let mut output = heapless::Deque::new();
        output
            .push_back(TaskOutput::Net(NetOutgoing::UdpListen {
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

impl Task<ExtIn, ExtOut, ChannelId, ChannelId, Event, Event> for EchoTask {
    const TYPE: u16 = 0;

    fn on_tick<'a>(
        &mut self,
        _now: Instant,
    ) -> Option<TaskOutput<'a, ExtOut, ChannelId, ChannelId, Event>> {
        self.output.pop_front()
    }

    fn on_event<'b>(
        &mut self,
        _now: Instant,
        input: TaskInput<'b, ExtIn, ChannelId, Event>,
    ) -> Option<TaskOutput<'b, ExtOut, ChannelId, ChannelId, Event>> {
        match input {
            TaskInput::Net(NetIncoming::UdpListenResult { bind, result }) => {
                log::info!("UdpListenResult: {} {:?}", bind, result);
                if let Ok((addr, slot)) = result {
                    self.local_addr = addr;
                    self.local_backend_slot = slot;
                    for _ in 0..self.cfg.brust_size {
                        self.output
                            .push_back(TaskOutput::Net(NetOutgoing::UdpPacket {
                                slot: self.local_backend_slot,
                                to: self.cfg.dest,
                                data: Buffer::Vec([0; 1000].to_vec()),
                            }))
                            .print_err2("Should push ok");
                    }
                }
                None
            }
            TaskInput::Net(NetIncoming::UdpPacket { from, slot, data }) => {
                self.count += 1;
                if self.count >= self.cfg.count {
                    log::info!("EchoTask done");
                    Some(TaskOutput::Destroy)
                } else {
                    Some(TaskOutput::Net(NetOutgoing::UdpPacket {
                        slot,
                        to: from,
                        data: Buffer::Ref(data),
                    }))
                }
            }
            _ => unreachable!("EchoTask only has NetIncoming variants"),
        }
    }

    fn pop_output<'a>(
        &mut self,
        _now: Instant,
    ) -> Option<TaskOutput<'a, ExtOut, ChannelId, ChannelId, Event>> {
        self.output.pop_front()
    }

    fn shutdown<'a>(
        &mut self,
        _now: Instant,
    ) -> Option<TaskOutput<'a, ExtOut, ChannelId, ChannelId, Event>> {
        log::info!("EchoTask shutdown");
        self.output
            .push_back(TaskOutput::Net(NetOutgoing::UdpUnlisten {
                slot: self.local_backend_slot,
            }))
            .print_err2("should not hapended");
        self.output
            .push_back(TaskOutput::Destroy)
            .print_err2("should not hapended");
        self.output.pop_front()
    }
}

struct EchoWorkerInner {
    worker: u16,
    tasks: TaskGroup<ExtIn, ExtOut, ChannelId, ChannelId, Event, Event, EchoTask, 16>,
}

impl WorkerInner<ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg> for EchoWorkerInner {
    fn tasks(&self) -> usize {
        self.tasks.tasks()
    }

    fn worker_index(&self) -> u16 {
        self.worker
    }

    fn build(worker: u16, _cfg: ICfg) -> Self {
        Self {
            worker,
            tasks: TaskGroup::new(worker),
        }
    }

    fn spawn(&mut self, _now: Instant, cfg: SCfg) {
        self.tasks.add_task(EchoTask::new(cfg));
    }

    fn on_tick<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        self.tasks.on_tick(now).map(|a| a.into())
    }

    fn on_event<'a>(
        &mut self,
        now: Instant,
        event: WorkerInnerInput<'a, ExtIn, ChannelId, Event>,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        match event {
            WorkerInnerInput::Task(owner, event) => self
                .tasks
                .on_event(now, TaskGroupInput(owner, event))
                .map(|a| a.into()),
            _ => unreachable!(),
        }
    }

    fn pop_output<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        self.tasks.pop_output(now).map(|a| a.into())
    }

    fn shutdown<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        self.tasks.shutdown(now).map(|a| a.into())
    }
}

fn main() {
    env_logger::init();
    println!("{}", std::mem::size_of::<EchoWorkerInner>());
    let mut controller =
        Controller::<ExtIn, ExtOut, EchoTaskCfg, ChannelId, Event, 1024>::default();
    controller.add_worker::<_, EchoWorkerInner, PollBackend<1024, 1024>>(
        Duration::from_secs(1),
        (),
        None,
    );
    controller.add_worker::<_, EchoWorkerInner, PollBackend<1024, 1024>>(
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
