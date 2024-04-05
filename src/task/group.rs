use std::{hash::Hash, marker::PhantomData, time::Instant};

use crate::{
    collections::DynamicVec, Task, TaskInput, TaskOutput, TaskSwitcher, WorkerInnerOutput,
};

pub trait TaskGroupOwner {
    fn build(index: usize) -> Self;
    fn task_index(&self) -> usize;
}

/// Represents the input of a task group.
pub struct TaskGroupInput<'a, Owner: TaskGroupOwner, ExtIn, ChannelId, Event>(
    pub Owner,
    pub TaskInput<'a, ExtIn, ChannelId, Event>,
);

/// Represents the output of a task group.
pub struct TaskGroupOutput<'a, Owner: TaskGroupOwner, ExtOut, ChannelIn, ChannelOut, Event>(
    pub Owner,
    pub TaskOutput<'a, ExtOut, ChannelIn, ChannelOut, Event>,
);

impl<
        'a,
        Owner: TaskGroupOwner,
        ExtOut,
        ChannelIn,
        ChannelOut,
        Event,
        IOwner: From<Owner>,
        IExtOut: From<ExtOut>,
        IChannel: From<ChannelIn> + From<ChannelOut>,
        IEvent: From<Event>,
        ISCfg,
    > From<TaskGroupOutput<'a, Owner, ExtOut, ChannelIn, ChannelOut, Event>>
    for WorkerInnerOutput<'a, IOwner, IExtOut, IChannel, IEvent, ISCfg>
{
    fn from(value: TaskGroupOutput<'a, Owner, ExtOut, ChannelIn, ChannelOut, Event>) -> Self {
        WorkerInnerOutput::Task(value.0.into(), value.1.convert_into())
    }
}

/// Represents a group of tasks.
pub struct TaskGroup<
    Owner: TaskGroupOwner,
    ExtIn,
    ExtOut,
    ChannelIn: Hash + Eq + PartialEq,
    ChannelOut,
    EventIn,
    EventOut,
    T: Task<ExtIn, ExtOut, ChannelIn, ChannelOut, EventIn, EventOut>,
    const STACK_SIZE: usize,
> {
    tasks: DynamicVec<Option<T>, STACK_SIZE>,
    _tmp: PhantomData<(
        Owner,
        ExtIn,
        ExtOut,
        ChannelIn,
        ChannelOut,
        EventIn,
        EventOut,
    )>,
    switcher: TaskSwitcher,
    destroy_list: DynamicVec<usize, STACK_SIZE>,
}

impl<
        Owner: TaskGroupOwner,
        ExtIn,
        ExtOut,
        ChannelIn: Hash + Eq + PartialEq,
        ChannelOut,
        EventIn,
        EventOut,
        T: Task<ExtIn, ExtOut, ChannelIn, ChannelOut, EventIn, EventOut>,
        const STACK_SIZE: usize,
    > TaskGroup<Owner, ExtIn, ExtOut, ChannelIn, ChannelOut, EventIn, EventOut, T, STACK_SIZE>
{
    /// Creates a new task group with the specified worker ID.
    pub fn new() -> Self {
        Self {
            tasks: DynamicVec::default(),
            _tmp: Default::default(),
            switcher: TaskSwitcher::new(0),
            destroy_list: DynamicVec::default(),
        }
    }

    /// Returns the number of tasks in the group.
    pub fn tasks(&self) -> usize {
        //count all task which not None
        let tasks = self.tasks.iter().filter(|x| x.is_some()).count();
        tasks
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
        self.switcher.set_tasks(self.tasks.len());
        self.tasks.len() - 1
    }

    /// The idea of this function is external will call it utils it returns None.
    /// If a task has output, we will return the output and save flag next_tick_index for next call.
    /// In the end of list, we will clear next_tick_index
    pub fn on_tick<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskGroupOutput<'a, Owner, ExtOut, ChannelIn, ChannelOut, EventOut>> {
        self.clear_destroyed_task();

        while let Some(index) = self.switcher.looper_current(now) {
            let task = match self.tasks.get_mut_or_panic(index) {
                Some(task) => task,
                None => {
                    self.switcher.looper_process(None::<()>);
                    continue;
                }
            };
            if let Some(out) = self.switcher.looper_process(task.on_tick(now)) {
                if let TaskOutput::Destroy = out {
                    self.destroy_list.push_safe(index);
                }
                let owner = Owner::build(index);
                return Some(TaskGroupOutput(owner, out));
            }
        }

        None
    }

    /// This function send an event to a task and return the output if the task has output.
    /// If the task has output, we will return the output and save flag last_input_index for next pop_last_input call.
    pub fn on_event<'a>(
        &mut self,
        now: Instant,
        input: TaskGroupInput<'a, Owner, ExtIn, ChannelIn, EventIn>,
    ) -> Option<TaskGroupOutput<'a, Owner, ExtOut, ChannelIn, ChannelOut, EventOut>> {
        let TaskGroupInput(owner, input) = input;
        let index = owner.task_index();
        let task = self
            .tasks
            .get_mut_or_panic(index)
            .as_mut()
            .expect("should have task");
        let out = task.on_event(now, input)?;
        self.switcher.queue_flag_task(index);
        if let TaskOutput::Destroy = out {
            self.destroy_list.push_safe(index);
        }
        Some(TaskGroupOutput(owner, out))
    }

    /// Retrieves the output from the last processed task input event.
    /// In SAN/IO we usually have some output when we receive an input event.
    /// External will call this function util it return None.
    /// We use last_input_index which is saved in previous on_input_event or on_input_tick and clear it after we got None.
    pub fn pop_output<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskGroupOutput<'a, Owner, ExtOut, ChannelIn, ChannelOut, EventOut>> {
        // We dont clear_destroyed_task here because have some case we have output after we received TaskOutput::Destroy the task.
        // We will clear_destroyed_task in next pop_output call.

        while let Some(index) = self.switcher.queue_current() {
            let owner = Owner::build(index);
            let slot = self
                .tasks
                .get_mut_or_panic(index)
                .as_mut()
                .expect("should have task");
            if let Some(out) = self.switcher.queue_process(slot.pop_output(now)) {
                if let TaskOutput::Destroy = out {
                    self.destroy_list.push_safe(index);
                }
                return Some(TaskGroupOutput(owner, out));
            }
        }
        None
    }

    /// Gracefully destroys the task group.
    pub fn shutdown<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskGroupOutput<'a, Owner, ExtOut, ChannelIn, ChannelOut, EventOut>> {
        while let Some(index) = self.switcher.looper_current(now) {
            log::info!("Group kill tasks {}/{}", index, self.switcher.tasks());
            //We only call each task single time
            self.switcher.looper_process(None::<()>);
            let task = match self.tasks.get_mut_or_panic(index) {
                Some(task) => task,
                None => {
                    continue;
                }
            };
            if let Some(out) = task.shutdown(now) {
                if let TaskOutput::Destroy = out {
                    self.destroy_list.push_safe(index);
                }
                let owner = Owner::build(index);
                return Some(TaskGroupOutput(owner, out));
            }
        }

        None
    }

    fn clear_destroyed_task(&mut self) {
        while let Some(index) = self.destroy_list.pop() {
            self.tasks.get_mut_or_panic(index).take();
        }
        while let Some(None) = self.tasks.last() {
            self.tasks.pop();
        }
        self.switcher.set_tasks(self.tasks.len());
    }
}
