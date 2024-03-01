use std::{hash::Hash, marker::PhantomData, time::Instant};

use crate::{collections::DynamicVec, Owner, Task, TaskInput, TaskOutput, WorkerInnerOutput};

/// Represents the input of a task group.
pub struct TaskGroupInput<'a, ChannelId, Event>(pub Owner, pub TaskInput<'a, ChannelId, Event>);

/// Represents the output of a task group.
pub struct TaskGroupOutput<'a, ChannelId, Event>(pub Owner, pub TaskOutput<'a, ChannelId, Event>);

impl<'a, ExtOut, ChannelId, Event, SCfg> Into<WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg>>
    for TaskGroupOutput<'a, ChannelId, Event>
{
    fn into(self) -> WorkerInnerOutput<'a, ExtOut, ChannelId, Event, SCfg> {
        WorkerInnerOutput::Task(self.0, self.1)
    }
}

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
    next_tick_index: Option<usize>,
    last_input_index: Option<usize>,
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
            next_tick_index: None,
            last_input_index: None,
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

    /// The idea of this function is external will call it utils it returns None.
    /// If a task has output, we will return the output and save flag next_tick_index for next call.
    /// In the end of list, we will clear next_tick_index
    pub fn on_input_tick<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskGroupOutput<'a, ChannelId, Event>> {
        self.clear_destroyed_task();

        let mut index = self.next_tick_index.unwrap_or(0);
        while index < self.tasks.len() {
            let task = match self.tasks.get_mut_or_panic(index) {
                Some(task) => task,
                None => {
                    index += 1;
                    continue;
                }
            };
            self.last_input_index = Some(index);
            self.next_tick_index = Some(index + 1);
            if let Some(out) = task.on_tick(now) {
                if let TaskOutput::Destroy = out {
                    self.destroy_list.push_safe(index);
                }
                let owner = Owner::task(self.worker, T::TYPE, index);
                return Some(TaskGroupOutput(owner, out));
            } else {
                index += 1;
            }
        }

        self.next_tick_index = None;
        self.last_input_index = None;

        None
    }

    /// This function send an event to a task and return the output if the task has output.
    /// If the task has output, we will return the output and save flag last_input_index for next pop_last_input call.
    pub fn on_input_event<'a>(
        &mut self,
        now: Instant,
        input: TaskGroupInput<'a, ChannelId, Event>,
    ) -> Option<TaskGroupOutput<'a, ChannelId, Event>> {
        let TaskGroupInput(owner, input) = input;
        let index = owner.task_index().expect("should have task index");
        let task = self
            .tasks
            .get_mut_or_panic(index)
            .as_mut()
            .expect("should have task");
        let out = task.on_input(now, input)?;
        self.last_input_index = Some(index);
        if let TaskOutput::Destroy = out {
            self.destroy_list.push_safe(index);
        }
        Some(TaskGroupOutput(owner, out))
    }

    /// Retrieves the output from the last processed task input event.
    /// In SAN/IO we usually have some output when we receive an input event.
    /// External will call this function util it return None.
    /// We use last_input_index which is saved in previous on_input_event or on_input_tick and clear it after we got None.
    pub fn pop_last_input<'a>(
        &mut self,
        now: Instant,
    ) -> Option<TaskGroupOutput<'a, ChannelId, Event>> {
        // We dont clear_destroyed_task here because have some case we have output after we received TaskOutput::Destroy the task.
        // We will clear_destroyed_task in next pop_output call.

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

    fn clear_destroyed_task(&mut self) {
        while let Some(index) = self.destroy_list.pop() {
            self.tasks.get_mut_or_panic(index).take();
        }
    }
}
