use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

use sans_io_runtime::{
    backend::MioBackend, Buffer, Controller, ErrorDebugger2, NetIncoming, NetOutgoing, Task,
    TaskGroup, TaskGroupInput, TaskGroupOutput, TaskGroupOutputsState, TaskInput, TaskOutput,
    WorkerInner, WorkerInnerInput, WorkerInnerOutput,
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
    output: heapless::Deque<TaskOutput<'static, ChannelId, Event>, 16>,
}

impl<const FAKE_TYPE: u16> EchoTask<FAKE_TYPE> {
    pub fn new(cfg: EchoTaskCfg) -> Self {
        log::info!("Create new echo client task in addr {}", cfg.dest);
        let mut output = heapless::Deque::new();
        output
            .push_back(TaskOutput::Net(NetOutgoing::UdpListen(SocketAddr::from((
                [127, 0, 0, 1],
                0,
            )))))
            .print_err2("should not hapended");
        Self {
            count: 0,
            cfg,
            local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            output,
        }
    }
}

impl<const FAKE_TYPE: u16> Task<ChannelId, Event> for EchoTask<FAKE_TYPE> {
    const TYPE: u16 = FAKE_TYPE;

    fn on_tick<'a>(&mut self, _now: Instant) -> Option<TaskOutput<'a, ChannelId, Event>> {
        None
    }

    fn on_input<'b>(
        &mut self,
        _now: Instant,
        input: TaskInput<'b, ChannelId, Event>,
    ) -> Option<TaskOutput<'b, ChannelId, Event>> {
        match input {
            TaskInput::Net(NetIncoming::UdpListenResult { bind, result }) => {
                log::info!("UdpListenResult: {} {:?}", bind, result);
                if let Ok(addr) = result {
                    self.local_addr = addr;
                    for _ in 0..self.cfg.brust_size {
                        self.output
                            .push_back(TaskOutput::Net(NetOutgoing::UdpPacket {
                                from: self.local_addr,
                                to: self.cfg.dest,
                                data: Buffer::Vec([0; 1000].to_vec()),
                            }))
                            .print_err2("Should push ok");
                    }
                }
                None
            }
            TaskInput::Net(NetIncoming::UdpPacket { from, to, data }) => {
                self.count += 1;
                if self.count >= self.cfg.count {
                    log::info!("EchoTask {} done", FAKE_TYPE);
                    Some(TaskOutput::Destroy)
                } else {
                    Some(TaskOutput::Net(NetOutgoing::UdpPacket {
                        from: to,
                        to: from,
                        data: Buffer::Ref(data),
                    }))
                }
            }
            _ => unreachable!("EchoTask only has NetIncoming variants"),
        }
    }

    fn pop_output<'a>(&mut self, _now: Instant) -> Option<TaskOutput<'a, ChannelId, Event>> {
        self.output.pop_front()
    }
}

struct EchoWorkerInner {
    worker: u16,
    echo_type1: TaskGroup<ChannelId, Event, EchoTask<0>, 16>,
    echo_type2: TaskGroup<ChannelId, Event, EchoTask<1>, 16>,
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

    fn on_input_tick<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        loop {
            match self.group_state.current()? {
                0 => match self.echo_type1.on_input_tick(now) {
                    Some(e) => {
                        return Some(e.into());
                    }
                    None => {
                        self.group_state.finish_current();
                    }
                },
                1 => match self.echo_type2.on_input_tick(now) {
                    Some(e) => {
                        return Some(e.into());
                    }
                    None => {
                        self.group_state.finish_current();
                    }
                },
                _ => unreachable!(),
            }
        }
    }

    fn on_input_event<'a>(
        &mut self,
        now: Instant,
        event: WorkerInnerInput<'a, ExtIn, ChannelId, Event>,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        match event {
            WorkerInnerInput::Task(owner, event) => match owner.group_id() {
                Some(0) => {
                    let TaskGroupOutput(owner, output) = self
                        .echo_type1
                        .on_input_event(now, TaskGroupInput(owner, event))?;
                    self.last_input_index = Some(1);
                    Some(WorkerInnerOutput::Task(owner, output))
                }
                Some(1) => {
                    let TaskGroupOutput(owner, output) = self
                        .echo_type2
                        .on_input_event(now, TaskGroupInput(owner, event))?;
                    self.last_input_index = Some(2);
                    Some(WorkerInnerOutput::Task(owner, output))
                }
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    }

    fn pop_last_input<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>> {
        match self.last_input_index? {
            0 => self.echo_type1.pop_last_input(now).map(|a| a.into()),
            1 => self.echo_type2.pop_last_input(now).map(|a| a.into()),
            _ => unreachable!(),
        }
    }
}

fn main() {
    env_logger::init();
    println!("{}", std::mem::size_of::<EchoWorkerInner>());
    let mut controller =
        Controller::<ExtIn, ExtOut, EchoTaskMultiCfg, ChannelId, Event, 1024>::new();
    controller.add_worker::<_, EchoWorkerInner, MioBackend<1024, 1024>>((), None);
    controller.add_worker::<_, EchoWorkerInner, MioBackend<1024, 1024>>((), None);

    for _i in 0..100 {
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
    loop {
        controller.process();
        std::thread::sleep(Duration::from_millis(100));
    }
}
