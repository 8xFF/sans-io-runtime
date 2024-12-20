use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use sans_io_runtime::{
    backend::PollBackend, group_owner_type, Controller, Task, TaskGroup, TaskSwitcher,
    TaskSwitcherBranch, TaskSwitcherChild, WorkerInner, WorkerInnerInput, WorkerInnerOutput,
};

type ICfg = ();

#[derive(Clone)]
enum Type1ExtIn {}

#[derive(Clone)]
enum Type2ExtIn {}

#[derive(Clone)]
enum Type1ExtOut {}

#[derive(Clone)]
enum Type2ExtOut {}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
enum Type1Channel {}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
enum Type2Channel {}

#[derive(Clone)]
enum Type1Event {}

#[derive(Clone)]
enum Type2Event {}

#[derive(Debug, Clone)]
struct Type1Cfg {}

#[derive(Debug, Clone)]
struct Type2Cfg {}

#[derive(convert_enum::From, convert_enum::TryInto, Clone)]
enum TestExtIn {
    Type1(Type1ExtIn),
    Type2(Type2ExtIn),
}

#[derive(convert_enum::From, convert_enum::TryInto, Clone)]
enum TestExtOut {
    Type1(Type1ExtOut),
    Type2(Type2ExtOut),
}

#[derive(Debug, Hash, PartialEq, Eq, convert_enum::From, convert_enum::TryInto, Clone, Copy)]
enum TestChannel {
    Type1(Type1Channel),
    Type2(Type2Channel),
}

#[derive(Debug, Clone, convert_enum::From, convert_enum::TryInto)]
enum TestSCfg {
    Type1(Type1Cfg),
    Type2(Type2Cfg),
}

#[derive(convert_enum::From, convert_enum::TryInto, Clone)]
enum TestEvent {
    Type1(Type1Event),
    Type2(Type2Event),
}

#[derive(Debug)]
struct Task1 {
    _cfg: Type1Cfg,
}

impl Task1 {
    fn new(_cfg: Type1Cfg) -> Self {
        Self { _cfg }
    }
}

impl Task<(), ()> for Task1 {
    fn on_tick(&mut self, _now: Instant) {}

    fn on_event(&mut self, _now: Instant, _input: ()) {}

    fn on_shutdown(&mut self, _now: Instant) {}
}

impl TaskSwitcherChild<()> for Task1 {
    type Time = Instant;

    fn empty_event(&self) -> () {
        ()
    }

    fn is_empty(&self) -> bool {
        true
    }

    fn pop_output(&mut self, _now: Instant) -> Option<()> {
        None
    }
}

#[derive(Debug)]
struct Task2 {
    _cfg: Type2Cfg,
}

impl Task2 {
    fn new(_cfg: Type2Cfg) -> Self {
        Self { _cfg }
    }
}

impl Task<(), ()> for Task2 {
    fn on_tick(&mut self, _now: Instant) {}

    fn on_event(&mut self, _now: Instant, _input: ()) {}

    fn on_shutdown(&mut self, _now: Instant) {}
}

impl TaskSwitcherChild<()> for Task2 {
    type Time = Instant;

    fn empty_event(&self) -> () {
        ()
    }

    fn is_empty(&self) -> bool {
        true
    }

    fn pop_output(&mut self, _now: Instant) -> Option<()> {
        None
    }
}

group_owner_type!(Type1Owner);
group_owner_type!(Type2Owner);

#[derive(convert_enum::From, Debug, Clone, Copy, PartialEq)]
enum OwnerType {
    Type1(Type1Owner),
    Type2(Type2Owner),
}

struct EchoWorkerInner {
    worker: u16,
    task_type1: TaskSwitcherBranch<TaskGroup<(), (), Task1, 16>, (usize, ())>,
    task_type2: TaskSwitcherBranch<TaskGroup<(), (), Task2, 16>, (usize, ())>,
    switcher: TaskSwitcher,
}

impl WorkerInner<OwnerType, TestExtIn, TestExtOut, TestChannel, TestEvent, ICfg, TestSCfg>
    for EchoWorkerInner
{
    fn tasks(&self) -> usize {
        self.task_type1.tasks() + self.task_type2.tasks()
    }

    fn is_empty(&self) -> bool {
        self.task_type1.is_empty() && self.task_type2.is_empty()
    }

    fn worker_index(&self) -> u16 {
        self.worker
    }

    fn build(worker: u16, _cfg: ICfg) -> Self {
        Self {
            worker,
            task_type1: TaskSwitcherBranch::default(0 as usize),
            task_type2: TaskSwitcherBranch::default(1 as usize),
            switcher: TaskSwitcher::new(2),
        }
    }

    fn spawn(&mut self, _now: Instant, cfg: TestSCfg) {
        match cfg {
            TestSCfg::Type1(cfg) => {
                self.task_type1
                    .input(&mut self.switcher)
                    .add_task(Task1::new(cfg));
            }
            TestSCfg::Type2(cfg) => {
                self.task_type2
                    .input(&mut self.switcher)
                    .add_task(Task2::new(cfg));
            }
        }
    }

    fn on_tick(&mut self, now: Instant) {
        self.task_type1.input(&mut self.switcher).on_tick(now);
        self.task_type2.input(&mut self.switcher).on_tick(now);
    }

    fn on_event(
        &mut self,
        _now: Instant,
        _event: WorkerInnerInput<OwnerType, TestExtIn, TestChannel, TestEvent>,
    ) {
    }

    fn pop_output(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<OwnerType, TestExtOut, TestChannel, TestEvent, TestSCfg>> {
        loop {
            match self.switcher.current()? {
                0 => {
                    if let Some(_e) = self.task_type1.input(&mut self.switcher).pop_output(now) {
                        // return Some(e.into());
                    }
                }
                1 => {
                    if let Some(_e) = self.task_type2.input(&mut self.switcher).pop_output(now) {
                        // return Some(e.into());
                    }
                }
                _ => unreachable!(),
            }
        }
    }

    fn on_shutdown(&mut self, now: Instant) {
        self.task_type1.input(&mut self.switcher).on_shutdown(now);
        self.task_type2.input(&mut self.switcher).on_shutdown(now);
    }
}

fn main() {
    env_logger::init();
    println!("{}", std::mem::size_of::<EchoWorkerInner>());
    let mut controller =
        Controller::<TestExtIn, TestExtOut, TestSCfg, TestChannel, TestEvent, 1024>::default();
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

    for _i in 0..10 {
        controller.spawn(TestSCfg::Type1(Type1Cfg {}));
        controller.spawn(TestSCfg::Type2(Type2Cfg {}));
    }
    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term))
        .expect("Should register hook");
    let mut shutdown_wait = 0;

    while controller.process().is_some() {
        if term.load(Ordering::Relaxed) {
            if shutdown_wait == 300 {
                log::warn!("Force shutdown");
                break;
            }
            shutdown_wait += 1;
            controller.shutdown();
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    log::info!("Server shutdown");
}
