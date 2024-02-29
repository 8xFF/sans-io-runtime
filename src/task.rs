///! Task system for handling network and bus events.
///!
///! Each task is a separate state machine that can receive network and bus events, and produce output events.
///! Tasks are grouped together into a task group, which is processed by a worker.
///! The worker is responsible for dispatching network and bus events to the appropriate task group, and for
///! processing the task groups.
///!
use std::{hash::Hash, marker::PhantomData, net::SocketAddr, ops::Deref, time::Instant};

use crate::{backend::BackendIncoming, bus::BusEvent, collections::DynamicVec, owner::Owner};

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

impl<'a> NetIncoming<'a> {
    pub fn from_backend(event: BackendIncoming, buf: &'a [u8]) -> Self {
        match event {
            BackendIncoming::UdpListenResult { bind, result } => {
                Self::UdpListenResult { bind, result }
            }
            BackendIncoming::UdpPacket { from, to, len } => {
                let data = &buf[..len];
                Self::UdpPacket { from, to, data }
            }
        }
    }
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

    /// Called each time the task is ticked. Default is 1ms.
    fn on_tick<'a>(&mut self, now: Instant) -> Option<TaskOutput<'a, ChannelId, Event>>;

    /// Called when an input event is received for the task.
    fn on_input<'a>(
        &mut self,
        now: Instant,
        input: TaskInput<'a, ChannelId, Event>,
    ) -> Option<TaskOutput<'a, ChannelId, Event>>;

    /// Retrieves the next output event from the task.
    fn pop_output<'a>(&mut self, now: Instant) -> Option<TaskOutput<'a, ChannelId, Event>>;
}

/// Represents the input of a task group.
pub struct TaskGroupInput<'a, ChannelId, Event>(pub Owner, pub TaskInput<'a, ChannelId, Event>);

/// Represents the output of a task group.
pub struct TaskGroupOutput<'a, ChannelId, Event>(pub Owner, pub TaskOutput<'a, ChannelId, Event>);

/// Represents a group of tasks.
pub struct TaskGroup<
    ChannelId: Hash + Eq + PartialEq,
    Event,
    T: Task<ChannelId, Event>,
    const STACK_SIZE: usize,
> {
    worker: u16,
    tasks: DynamicVec<Option<T>, STACK_SIZE>,
    _tmp: PhantomData<(ChannelId, Event)>,
    last_input_index: Option<usize>,
    pop_output_index: usize,
    destroy_list: DynamicVec<usize, STACK_SIZE>,
}

impl<
        ChannelId: Hash + Eq + PartialEq,
        Event,
        T: Task<ChannelId, Event>,
        const MAX_TASKS: usize,
    > TaskGroup<ChannelId, Event, T, MAX_TASKS>
{
    /// Creates a new task group with the specified worker ID.
    pub fn new(worker: u16) -> Self {
        Self {
            worker,
            tasks: DynamicVec::new(),
            _tmp: PhantomData::default(),
            last_input_index: None,
            pop_output_index: 0,
            destroy_list: DynamicVec::new(),
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

    pub fn on_input_tick<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskGroupOutput<'a, ChannelId, Event>> {
        let mut index = self.last_input_index.unwrap_or(0);
        while index < self.tasks.len() {
            let task = match self.tasks.get_mut_or_panic(index) {
                Some(task) => task,
                None => {
                    index += 1;
                    continue;
                }
            };
            self.last_input_index = Some(index);
            index += 1;
            if let Some(out) = task.on_tick(now) {
                if let TaskOutput::Destroy = out {
                    self.destroy_list.push_safe(index);
                }
                return Some(TaskGroupOutput(
                    Owner::task(self.worker, T::TYPE, index),
                    out,
                ));
            }
        }

        None
    }

    pub fn on_input_event<'a>(
        &mut self,
        now: Instant,
        input: TaskGroupInput<'a, ChannelId, Event>,
    ) -> Option<TaskGroupOutput<'a, ChannelId, Event>> {
        self.clear_destroyed_task();
        let TaskGroupInput(owner, input) = input;
        let index = owner.task_index().expect("should have task index");
        self.last_input_index = Some(index);
        let task = self
            .tasks
            .get_mut_or_panic(index)
            .as_mut()
            .expect("should have task");
        let out = task.on_input(now, input)?;
        if let TaskOutput::Destroy = out {
            self.destroy_list.push_safe(index);
        }
        Some(TaskGroupOutput(owner, out))
    }

    /// Retrieves the output from the last processed task input event.
    /// In SAN/IO we ussually have some output when we receive an input event.
    pub fn pop_last_input<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskGroupOutput<'a, ChannelId, Event>> {
        let index = self.last_input_index?;
        let owner = Owner::task(self.worker, T::TYPE, index);
        let slot = self.tasks.get_mut_or_panic(index).as_mut();
        let out = match slot.expect("should have task").pop_output(now) {
            Some(output) => output,
            None => {
                self.last_input_index = None;
                return None;
            }
        };
        if let TaskOutput::Destroy = out {
            self.destroy_list.push_safe(index);
        }
        Some(TaskGroupOutput(owner, out))
    }

    /// Retrieves the next output event from the task group.
    pub fn pop_output<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskGroupOutput<'a, ChannelId, Event>> {
        self.clear_destroyed_task();

        let tasks = &mut self.tasks;
        while self.pop_output_index < tasks.len() {
            let task = match tasks.get_mut_or_panic(self.pop_output_index) {
                Some(task) => task,
                None => {
                    self.pop_output_index += 1;
                    continue;
                }
            };
            match task.pop_output(now) {
                Some(output) => {
                    if let TaskOutput::Destroy = output {
                        self.destroy_list.push_safe(self.pop_output_index);
                    }
                    return Some(TaskGroupOutput(
                        Owner::task(self.worker, T::TYPE, self.pop_output_index),
                        output,
                    ));
                }
                None => {
                    self.pop_output_index += 1;
                }
            }
        }

        self.pop_output_index = 0;
        None
    }

    fn clear_destroyed_task(&mut self) {
        while let Some(index) = self.destroy_list.pop() {
            self.tasks.get_mut_or_panic(index).take();
        }
    }
}
