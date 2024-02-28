///! Task system for handling network and bus events.
///!
///! Each task is a separate state machine that can receive network and bus events, and produce output events.
///! Tasks are grouped together into a task group, which is processed by a worker.
///! The worker is responsible for dispatching network and bus events to the appropriate task group, and for
///! processing the task groups.
///!
use std::{hash::Hash, net::SocketAddr, ops::Deref, time::Instant};

use crate::{
    bus::BusEvent,
    collections::{DynamicDeque, DynamicVec},
    owner::Owner,
    ErrorDebugger2, WorkerCtx,
};

/// Represents an incoming network event.
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

pub enum Buffer<'a> {
    Ref(&'a [u8]),
    Vec(Vec<u8>),
}

impl<'a> Deref for Buffer<'a> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        match self {
            Buffer::Ref(data) => data,
            Buffer::Vec(data) => data,
        }
    }
}

/// Represents an outgoing network event.
pub enum NetOutgoing<'a> {
    UdpListen(SocketAddr),
    UdpPacket {
        from: SocketAddr,
        to: SocketAddr,
        data: Buffer<'a>,
    },
}

/// Represents an input event for a task.
pub enum TaskInput<'a, ChannelId, Event> {
    Net(NetIncoming<'a>),
    Bus(ChannelId, Event),
}

/// Represents an output event for a task.
pub enum TaskOutput<'a, ChannelId, Event> {
    Net(NetOutgoing<'a>),
    Bus(BusEvent<ChannelId, Event>),
    Destroy,
}

/// Represents a task.
pub trait Task<ChannelId, Event> {
    /// The type identifier for the task.
    const TYPE: u16;

    /// Called on each tick of the task.
    fn on_tick(&mut self, now: Instant);

    /// Called when an input event is received for the task.
    fn on_input<'a>(&mut self, now: Instant, input: TaskInput<'a, ChannelId, Event>);

    /// Retrieves the next output event from the task.
    fn pop_output(&mut self, now: Instant) -> Option<TaskOutput<'_, ChannelId, Event>>;
}

/// Represents the output of a task group.
pub enum TaskGroupOutput<ChannelId, EventOut> {
    Bus(Owner, BusEvent<ChannelId, EventOut>),
    DestroyOwner(Owner),
}

/// Represents a group of tasks.
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
    /// Creates a new task group with the specified worker ID.
    pub fn new(worker: u16) -> Self {
        Self {
            worker,
            tasks: DynamicVec::new(),
            output: DynamicDeque::new(),
        }
    }

    /// Returns the number of tasks in the group.
    pub fn tasks(&self) -> usize {
        self.tasks.len()
    }

    /// Adds a task to the group.
    pub fn add_task(&mut self, task: T) -> usize {
        for (index, slot) in self.tasks.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(task);
                return index;
            }
        }

        self.tasks.push_safe(Some(task));
        self.tasks.len() - 1
    }

    /// Handles a bus event for the task group.
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

    /// Handles a network event for the task group.
    pub fn on_net(&mut self, now: Instant, owner: Owner, net: NetIncoming) {
        self.tasks
            .get_mut_or_panic(owner.task_index().expect("on_bus only allow task owner"))
            .as_mut()
            .expect("should have task")
            .on_input(now, TaskInput::Net(net));
    }

    /// Processes the tasks in the group.
    pub fn on_tick(&mut self, now: Instant, ctx: &mut WorkerCtx<'_>) {
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

    /// Retrieves the next output event from the task group.
    pub fn pop_output(&mut self) -> Option<TaskGroupOutput<ChannelId, Event>> {
        self.output.pop_front()
    }
}
