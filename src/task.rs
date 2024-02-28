use std::{hash::Hash, net::SocketAddr, time::Instant};

use crate::{
    bus::BusEvent,
    collections::{DynamicDeque, DynamicVec},
    owner::Owner,
    ErrorDebugger2, WorkerCtx,
};

pub enum NetIncoming<'a> {
    UdpListenResult {
        bind: SocketAddr,
        result: Result<SocketAddr, std::io::Error>,
    },
    UdpPacket {
        from: SocketAddr,
        to: SocketAddr,
        data: &'a [u8],
    },
}

pub enum NetOutgoing<'a> {
    UdpListen(SocketAddr),
    UdpPacket {
        from: SocketAddr,
        to: SocketAddr,
        data: &'a [u8],
    },
}

pub enum TaskInput<'a, ChannelId, Event> {
    Net(NetIncoming<'a>),
    Bus(ChannelId, Event),
}

pub enum TaskOutput<'a, ChannelId, Event> {
    Net(NetOutgoing<'a>),
    Bus(BusEvent<ChannelId, Event>),
    Destroy,
}

pub trait Task<ChannelId, Event> {
    const TYPE: u16;
    fn on_tick(&mut self, now: Instant);
    fn on_input<'a>(&mut self, now: Instant, input: TaskInput<'a, ChannelId, Event>);
    fn pop_output(&mut self, now: Instant) -> Option<TaskOutput<'_, ChannelId, Event>>;
}

pub enum TaskGroupOutput<ChannelId, EventOut> {
    Bus(Owner, BusEvent<ChannelId, EventOut>),
    DestroyOwner(Owner),
}

pub struct TaskGroup<
    ChannelId: Hash + Eq + PartialEq,
    Event,
    T: Task<ChannelId, Event>,
    const STACK_SIZE: usize,
    const QUEUE_SIZE: usize,
> {
    worker: u16,
    tasks: DynamicVec<Option<T>, STACK_SIZE>,
    output: DynamicDeque<TaskGroupOutput<ChannelId, Event>, QUEUE_SIZE>,
}

impl<
        ChannelId: Hash + Eq + PartialEq,
        Event,
        T: Task<ChannelId, Event>,
        const MAX_TASKS: usize,
        const QUEUE_SIZE: usize,
    > TaskGroup<ChannelId, Event, T, MAX_TASKS, QUEUE_SIZE>
{
    pub fn new(worker: u16) -> Self {
        Self {
            worker,
            tasks: DynamicVec::new(),
            output: DynamicDeque::new(),
        }
    }

    pub fn tasks(&self) -> usize {
        self.tasks.len()
    }

    pub fn add_task(&mut self, task: T) {
        for slot in self.tasks.iter_mut() {
            if slot.is_none() {
                *slot = Some(task);
                return;
            }
        }

        self.tasks.push_safe(Some(task));
    }

    pub fn on_bus(
        &mut self,
        now: Instant,
        _ctx: &mut WorkerCtx<'_>,
        owner: Owner,
        channel_id: ChannelId,
        event: Event,
    ) {
        self.tasks
            .get_mut_or_panic(owner.task_index().expect("on_bus only allow task owner"))
            .as_mut()
            .expect("should have task")
            .on_input(now, TaskInput::Bus(channel_id, event));
    }

    pub fn on_net(&mut self, now: Instant, owner: Owner, net: NetIncoming) {
        self.tasks
            .get_mut_or_panic(owner.task_index().expect("on_bus only allow task owner"))
            .as_mut()
            .expect("should have task")
            .on_input(now, TaskInput::Net(net));
    }

    pub fn inner_process(&mut self, now: Instant, ctx: &mut WorkerCtx<'_>) {
        for (index, slot) in self.tasks.iter_mut().enumerate() {
            let task = match slot {
                Some(task) => task,
                None => continue,
            };
            task.on_tick(now);
            let mut destroyed = false;
            while let Some(output) = task.pop_output(now) {
                match output {
                    TaskOutput::Net(out) => {
                        ctx.backend
                            .on_action(Owner::task(self.worker, T::TYPE, index), out);
                    }
                    TaskOutput::Bus(event) => {
                        self.output
                            .push_back(
                                event.high_priority(),
                                TaskGroupOutput::Bus(
                                    Owner::task(self.worker, T::TYPE, index),
                                    event,
                                ),
                            )
                            .print_err2("queue full");
                    }
                    TaskOutput::Destroy => {
                        destroyed = true;
                    }
                }
            }
            if destroyed {
                *slot = None;
                self.output
                    .push_back_safe(TaskGroupOutput::DestroyOwner(Owner::task(
                        self.worker,
                        T::TYPE,
                        index,
                    )));
            }
        }
    }

    pub fn pop_output(&mut self) -> Option<TaskGroupOutput<ChannelId, Event>> {
        self.output.pop_front()
    }
}
