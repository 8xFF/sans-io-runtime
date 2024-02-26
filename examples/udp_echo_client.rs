use std::{
    collections::VecDeque,
    net::SocketAddr,
    time::{Duration, Instant},
};

use sans_io_runtime::{
    Controller, MioBackend, NetIncoming, NetOutgoing, Task, TaskGroup, TaskGroupOutput, TaskInput,
    TaskOutput, WorkerInner,
};

type ExtIn = ();
type ExtOut = ();
type ICfg = ();
type SCfg = EchoTaskMultiCfg;

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

enum EchoTaskInQueue {
    UdpListen(SocketAddr),
    SendUdpPacket {
        from: SocketAddr,
        to: SocketAddr,
        buf_index: usize,
        len: usize,
    },
    Destroy,
}

struct EchoTask<const FakeType: u16> {
    count: usize,
    cfg: EchoTaskCfg,
    buffers: [[u8; 1500]; 512],
    buffer_index: usize,
    local_addr: SocketAddr,
    output: VecDeque<EchoTaskInQueue>,
}

impl<const FakeType: u16> EchoTask<FakeType> {
    pub fn new(cfg: EchoTaskCfg) -> Self {
        log::info!("Create new echo client task in addr {}", cfg.dest);
        Self {
            count: 0,
            cfg,
            buffers: [[0; 1500]; 512],
            buffer_index: 0,
            local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            output: VecDeque::from([EchoTaskInQueue::UdpListen(SocketAddr::from((
                [127, 0, 0, 1],
                0,
            )))]),
        }
    }
}

impl<const FakeType: u16> Task<ExtIn, ExtOut> for EchoTask<FakeType> {
    const TYPE: u16 = FakeType;

    fn on_tick(&mut self, _now: Instant) {}

    fn on_input<'b>(&mut self, _now: Instant, input: TaskInput<'b, ExtIn>) {
        match input {
            TaskInput::Net(NetIncoming::UdpListenResult { bind, result }) => {
                log::info!("UdpListenResult: {} {:?}", bind, result);
                if let Ok(addr) = result {
                    self.local_addr = addr;
                    for _ in 0..self.cfg.brust_size {
                        self.output.push_back(EchoTaskInQueue::SendUdpPacket {
                            from: self.local_addr,
                            to: self.cfg.dest,
                            buf_index: self.buffer_index,
                            len: 1000,
                        });
                        self.buffer_index = (self.buffer_index + 1) % self.buffers.len();
                    }
                }
            }
            TaskInput::Net(NetIncoming::UdpPacket { from, to, data }) => {
                self.count += 1;
                assert!(data.len() <= 1500, "data too large");
                let buffer_index = self.buffer_index;
                self.buffer_index = (self.buffer_index + 1) % self.buffers.len();
                self.buffers[buffer_index][0..data.len()].copy_from_slice(data);

                self.output.push_back(EchoTaskInQueue::SendUdpPacket {
                    from: to,
                    to: from,
                    buf_index: buffer_index,
                    len: data.len(),
                });

                if self.count == self.cfg.count {
                    log::info!("Destroying task");
                    self.output.push_back(EchoTaskInQueue::Destroy);
                }
            }
            _ => unreachable!("EchoTask only has NetIncoming variants"),
        }
    }

    fn pop_output(&mut self, _now: Instant) -> Option<TaskOutput<'_, ExtOut>> {
        let out = self.output.pop_front()?;
        match out {
            EchoTaskInQueue::UdpListen(bind) => Some(TaskOutput::Net(NetOutgoing::UdpListen(bind))),
            EchoTaskInQueue::SendUdpPacket {
                from,
                to,
                buf_index,
                len,
            } => Some(TaskOutput::Net(NetOutgoing::UdpPacket {
                from,
                to,
                data: &self.buffers[buf_index][0..len],
            })),
            EchoTaskInQueue::Destroy => Some(TaskOutput::Destroy),
        }
    }
}

struct EchoWorkerInner {
    worker: u16,
    echo_type1: TaskGroup<ExtIn, ExtOut, EchoTask<1>>,
    echo_type2: TaskGroup<ExtIn, ExtOut, EchoTask<2>>,
}

impl WorkerInner<ExtIn, ExtOut, ICfg, SCfg> for EchoWorkerInner {
    fn tasks(&self) -> usize {
        self.echo_type1.tasks() + self.echo_type2.tasks()
    }

    fn build(worker: u16, _cfg: ()) -> Self {
        Self {
            worker,
            echo_type1: TaskGroup::new(worker),
            echo_type2: TaskGroup::new(worker),
        }
    }

    fn spawn(&mut self, _now: Instant, _ctx: &mut sans_io_runtime::WorkerCtx<'_>, cfg: SCfg) {
        match cfg {
            EchoTaskMultiCfg::Type1(cfg) => {
                self.echo_type1.add_task(EchoTask::new(cfg));
            }
            EchoTaskMultiCfg::Type2(cfg) => {
                self.echo_type2.add_task(EchoTask::new(cfg));
            }
        }
    }

    fn on_ext(&mut self, now: Instant, ctx: &mut sans_io_runtime::WorkerCtx<'_>, ext: ExtIn) {
        todo!()
    }

    fn on_net(&mut self, now: Instant, owner: sans_io_runtime::Owner, net: NetIncoming) {
        match owner.group_id() {
            Some(1) => {
                self.echo_type1.on_net(now, owner, net);
            }
            Some(2) => {
                self.echo_type2.on_net(now, owner, net);
            }
            _ => unreachable!(),
        }
    }

    fn inner_process(&mut self, now: Instant, ctx: &mut sans_io_runtime::WorkerCtx<'_>) {
        self.echo_type1.inner_process(now, ctx);
        self.echo_type2.inner_process(now, ctx);
    }

    fn pop_output(&mut self) -> Option<sans_io_runtime::WorkerInnerOutput<'_, ExtOut>> {
        if let Some(event) = self.echo_type1.pop_output() {
            match event {
                TaskGroupOutput::Net(net) => Some(sans_io_runtime::WorkerInnerOutput::Net(
                    net,
                    sans_io_runtime::Owner::worker(self.worker),
                )),
                TaskGroupOutput::External(_) => None,
            }
        } else if let Some(event) = self.echo_type2.pop_output() {
            match event {
                TaskGroupOutput::Net(net) => Some(sans_io_runtime::WorkerInnerOutput::Net(
                    net,
                    sans_io_runtime::Owner::worker(self.worker),
                )),
                TaskGroupOutput::External(_) => None,
            }
        } else {
            None
        }
    }
}

fn main() {
    env_logger::init();
    let mut controller = Controller::<ExtIn, ExtOut, EchoTaskMultiCfg>::new();
    controller.add_worker::<_, EchoWorkerInner, MioBackend>(());
    controller.add_worker::<_, EchoWorkerInner, MioBackend>(());

    for i in 0..10 {
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
