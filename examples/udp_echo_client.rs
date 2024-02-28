use std::{
    net::SocketAddr,
    time::{Duration, Instant},
};

use sans_io_runtime::{
    backend::MioBackend, Buffer, Controller, ErrorDebugger, NetIncoming, NetOutgoing, Owner, Task,
    TaskGroup, TaskGroupOutput, TaskInput, TaskOutput, WorkerCtx, WorkerInner, WorkerInnerOutput,
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

#[derive(Debug)]
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

struct EchoTask<const FAKE_TYPE: u16> {
    count: usize,
    cfg: EchoTaskCfg,
    buffers: [[u8; 1500]; 16],
    buffer_index: usize,
    local_addr: SocketAddr,
    output: heapless::Deque<EchoTaskInQueue, 16>,
}

impl<const FAKE_TYPE: u16> EchoTask<FAKE_TYPE> {
    pub fn new(cfg: EchoTaskCfg) -> Self {
        log::info!("Create new echo client task in addr {}", cfg.dest);
        let mut output = heapless::Deque::new();
        output
            .push_back(EchoTaskInQueue::UdpListen(SocketAddr::from((
                [127, 0, 0, 1],
                0,
            ))))
            .expect("should not happend");
        Self {
            count: 0,
            cfg,
            buffers: [[0; 1500]; 16],
            buffer_index: 0,
            local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            output,
        }
    }
}

impl<const FAKE_TYPE: u16> Task<ChannelId, Event> for EchoTask<FAKE_TYPE> {
    const TYPE: u16 = FAKE_TYPE;

    fn on_tick(&mut self, _now: Instant) {}

    fn on_input<'b>(&mut self, _now: Instant, input: TaskInput<'b, ChannelId, Event>) {
        match input {
            TaskInput::Net(NetIncoming::UdpListenResult { bind, result }) => {
                log::info!("UdpListenResult: {} {:?}", bind, result);
                if let Ok(addr) = result {
                    self.local_addr = addr;
                    for _ in 0..self.cfg.brust_size {
                        self.output
                            .push_back(EchoTaskInQueue::SendUdpPacket {
                                from: self.local_addr,
                                to: self.cfg.dest,
                                buf_index: self.buffer_index,
                                len: 1000,
                            })
                            .print_err("Output queue full");
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

                self.output
                    .push_back(EchoTaskInQueue::SendUdpPacket {
                        from: to,
                        to: from,
                        buf_index: buffer_index,
                        len: data.len(),
                    })
                    .print_err("Output queue full");

                if self.count == self.cfg.count {
                    log::info!("Destroying task");
                    self.output
                        .push_back(EchoTaskInQueue::Destroy)
                        .print_err("Output queue full");
                }
            }
            _ => unreachable!("EchoTask only has NetIncoming variants"),
        }
    }

    fn pop_output(&mut self, _now: Instant) -> Option<TaskOutput<'_, ChannelId, Event>> {
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
                data: Buffer::Ref(&self.buffers[buf_index][0..len]),
            })),
            EchoTaskInQueue::Destroy => Some(TaskOutput::Destroy),
        }
    }
}

struct EchoWorkerInner {
    worker: u16,
    echo_type1: TaskGroup<ChannelId, Event, EchoTask<1>, 16, 1024>,
    echo_type2: TaskGroup<ChannelId, Event, EchoTask<2>, 16, 1024>,
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
        }
    }

    fn spawn(&mut self, _now: Instant, _ctx: &mut WorkerCtx<'_>, cfg: SCfg) {
        match cfg {
            EchoTaskMultiCfg::Type1(cfg) => {
                self.echo_type1.add_task(EchoTask::new(cfg));
            }
            EchoTaskMultiCfg::Type2(cfg) => {
                self.echo_type2.add_task(EchoTask::new(cfg));
            }
        }
    }

    fn on_ext(&mut self, _now: Instant, _ctx: &mut WorkerCtx<'_>, _ext: ExtIn) {
        todo!()
    }

    fn on_bus(
        &mut self,
        _now: Instant,
        _ctx: &mut WorkerCtx<'_>,
        _owner: Owner,
        _channel_id: ChannelId,
        _event: Event,
    ) {
    }

    fn on_net(&mut self, now: Instant, owner: Owner, net: NetIncoming) {
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

    fn inner_process(&mut self, now: Instant, ctx: &mut WorkerCtx<'_>) {
        self.echo_type1.on_tick(now, ctx);
        self.echo_type2.on_tick(now, ctx);
    }

    fn pop_output(&mut self) -> Option<WorkerInnerOutput<'_, ExtOut, ChannelId, Event>> {
        if let Some(event) = self.echo_type1.pop_output() {
            match event {
                TaskGroupOutput::Bus(owner, event) => Some(WorkerInnerOutput::Bus(owner, event)),
                TaskGroupOutput::DestroyOwner(owner) => {
                    Some(WorkerInnerOutput::DestroyOwner(owner))
                }
            }
        } else if let Some(event) = self.echo_type2.pop_output() {
            match event {
                TaskGroupOutput::Bus(owner, event) => Some(WorkerInnerOutput::Bus(owner, event)),
                TaskGroupOutput::DestroyOwner(owner) => {
                    Some(WorkerInnerOutput::DestroyOwner(owner))
                }
            }
        } else {
            None
        }
    }
}

fn main() {
    env_logger::init();
    println!("{}", std::mem::size_of::<EchoWorkerInner>());
    let mut controller =
        Controller::<ExtIn, ExtOut, EchoTaskMultiCfg, ChannelId, Event, 1024>::new();
    controller.add_worker::<_, EchoWorkerInner, MioBackend<256, 1024>>((), None);
    controller.add_worker::<_, EchoWorkerInner, MioBackend<256, 1024>>((), None);

    for _i in 0..100 {
        controller.spawn(EchoTaskMultiCfg::Type1(EchoTaskCfg {
            count: 10000,
            brust_size: 1,
            dest: SocketAddr::from(([127, 0, 0, 1], 10001)),
        }));
        controller.spawn(EchoTaskMultiCfg::Type2(EchoTaskCfg {
            count: 100000,
            brust_size: 1,
            dest: SocketAddr::from(([127, 0, 0, 1], 10002)),
        }));
    }
    loop {
        controller.process();
        std::thread::sleep(Duration::from_millis(100));
    }
}
