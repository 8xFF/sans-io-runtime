use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use sans_io_runtime::{
    backend::MioBackend, Buffer, Controller, ErrorDebugger2, NetIncoming, NetOutgoing, Task,
    TaskGroup, TaskGroupInput, TaskGroupOutputsState, TaskInput, TaskOutput, WorkerInner,
    WorkerInnerInput, WorkerInnerOutput,
};

type ExtIn = ();
type ExtOut = ();
type ICfg = ();
type SCfg = EchoTaskMultiCfg;
type ChannelId = ();
type Event = ();

#[derive(Debug, Clone)]
struct EchoTaskCfg {
    count: usize,
    dest: SocketAddr,
    brust_size: usize,
}

#[derive(Debug, Clone)]
enum EchoTaskMultiCfg {
    Type1(EchoTaskCfg),
    Type2(EchoTaskCfg),
}

struct EchoTask<const FAKE_TYPE: u16> {
    count: usize,
    cfg: EchoTaskCfg,
    local_addr: SocketAddr,
    local_backend_slot: usize,
    output: heapless::Deque<TaskOutput<'static, ChannelId, ChannelId, Event>, 16>,
}

impl<const FAKE_TYPE: u16> EchoTask<FAKE_TYPE> {
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

impl<const FAKE_TYPE: u16> Task<ChannelId, ChannelId, Event, Event> for EchoTask<FAKE_TYPE> {
    const TYPE: u16 = FAKE_TYPE;

    fn on_tick<'a>(
        &mut self,
        _now: Instant,
    ) -> Option<TaskOutput<'a, ChannelId, ChannelId, Event>> {
        self.output.pop_front()
    }

    fn on_event<'b>(
        &mut self,
        _now: Instant,
        input: TaskInput<'b, ChannelId, Event>,
    ) -> Option<TaskOutput<'b, ChannelId, ChannelId, Event>> {
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
                    log::info!("EchoTask {} done", FAKE_TYPE);
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
    ) -> Option<TaskOutput<'a, ChannelId, ChannelId, Event>> {
        self.output.pop_front()
    }

    fn shutdown<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskOutput<'a, ChannelId, ChannelId, Event>> {
        log::info!("EchoTask {} shutdown", FAKE_TYPE);
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
    echo_type1: TaskGroup<ChannelId, ChannelId, Event, Event, EchoTask<0>, 16>,
    echo_type2: TaskGroup<ChannelId, ChannelId, Event, Event, EchoTask<1>, 16>,
    group_state: TaskGroupOutputsState<2>,
    last_input_index: Option<u16>,
}

impl WorkerInner<ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg> for EchoWorkerInner {
    fn tasks(&self) -> usize {
        self.echo_type1.tasks() + self.echo_type2.tasks()
    }

    fn worker_index(&self) -> u16 {
        self.worker
    }

    fn build(worker: u16, _cfg: ICfg) -> Self {
        Self {
            worker,
            echo_type1: TaskGroup::new(worker),
            echo_type2: TaskGroup::new(worker),
            last_input_index: None,
            group_state: TaskGroupOutputsState::default(),
        }
    }

    fn spawn(&mut self, _now: Instant, cfg: SCfg) {
        match cfg {
            EchoTaskMultiCfg::Type1(cfg) => {
                self.echo_type1.add_task(EchoTask::new(cfg));
            }
            EchoTaskMultiCfg::Type2(cfg) => {
                self.echo_type2.add_task(EchoTask::new(cfg));
            }
        }
    }

    fn on_tick<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        let gs = &mut self.group_state;
        loop {
            match gs.current()? {
                0 => {
                    if let Some(e) = gs.process(self.echo_type1.on_tick(now)) {
                        return Some(e.into());
                    }
                }
                1 => {
                    if let Some(e) = gs.process(self.echo_type2.on_tick(now)) {
                        return Some(e.into());
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    fn on_event<'a>(
        &mut self,
        now: Instant,
        event: WorkerInnerInput<'a, ExtIn, ChannelId, Event>,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        match event {
            WorkerInnerInput::Task(owner, event) => match owner.group_id() {
                Some(0) => {
                    let res = self
                        .echo_type1
                        .on_event(now, TaskGroupInput(owner, event))?;
                    self.last_input_index = Some(1);
                    Some(res.into())
                }
                Some(1) => {
                    let res = self
                        .echo_type2
                        .on_event(now, TaskGroupInput(owner, event))?;
                    self.last_input_index = Some(2);
                    Some(res.into())
                }
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    }

    fn pop_output<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        match self.last_input_index? {
            0 => self.echo_type1.pop_output(now).map(|a| a.into()),
            1 => self.echo_type2.pop_output(now).map(|a| a.into()),
            _ => unreachable!(),
        }
    }

    fn shutdown<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        let gs = &mut self.group_state;
        loop {
            match gs.current()? {
                0 => {
                    if let Some(e) = gs.process(self.echo_type1.shutdown(now)) {
                        return Some(e.into());
                    }
                }
                1 => {
                    if let Some(e) = gs.process(self.echo_type2.shutdown(now)) {
                        return Some(e.into());
                    }
                }
                _ => unreachable!(),
            }
        }
    }
}

fn main() {
    env_logger::init();
    println!("{}", std::mem::size_of::<EchoWorkerInner>());
    let mut controller =
        Controller::<ExtIn, ExtOut, EchoTaskMultiCfg, ChannelId, Event, 1024>::default();
    controller.add_worker::<_, EchoWorkerInner, MioBackend<1024, 1024>>((), None);
    controller.add_worker::<_, EchoWorkerInner, MioBackend<1024, 1024>>((), None);

    for _i in 0..2 {
        controller.spawn(EchoTaskMultiCfg::Type1(EchoTaskCfg {
            count: 1000,
            brust_size: 1,
            dest: SocketAddr::from(([127, 0, 0, 1], 10001)),
        }));
        controller.spawn(EchoTaskMultiCfg::Type2(EchoTaskCfg {
            count: 10000,
            brust_size: 1,
            dest: SocketAddr::from(([127, 0, 0, 1], 10002)),
        }));
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
