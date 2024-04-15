use std::{marker::PhantomData, time::Instant};

use crate::{collections::DynamicVec, Task, TaskSwitcher};

/// Represents a group of tasks.
pub struct TaskGroup<In, Out, T: Task<In, Out>, const STACK_SIZE: usize> {
    tasks: DynamicVec<Option<T>, STACK_SIZE>,
    switcher: TaskSwitcher,
    _tmp: PhantomData<(In, Out)>,
}

impl<In, Out, T: Task<In, Out>, const STACK_SIZE: usize> Default
    for TaskGroup<In, Out, T, STACK_SIZE>
{
    /// Creates a new task group with the specified worker ID.
    fn default() -> Self {
        Self {
            tasks: DynamicVec::default(),
            _tmp: Default::default(),
            switcher: TaskSwitcher::new(0),
        }
    }
}

impl<In, Out, T: Task<In, Out>, const STACK_SIZE: usize> TaskGroup<In, Out, T, STACK_SIZE> {
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

    /// Remove a task from the group
    pub fn remove_task(&mut self, index: usize) {
        self.tasks.get_mut_or_panic(index).take();
        while let Some(None) = self.tasks.last() {
            self.tasks.pop();
        }
        self.switcher.set_tasks(self.tasks.len());
    }

    /// The idea of this function is external will call it utils it returns None.
    /// If a task has output, we will return the output and save flag next_tick_index for next call.
    /// In the end of list, we will clear next_tick_index
    pub fn on_tick(&mut self, now: Instant) -> Option<(usize, Out)> {
        while let Some(index) = self.switcher.looper_current(now) {
            let task = match self.tasks.get_mut(index) {
                Some(Some(task)) => task,
                _ => {
                    self.switcher.looper_process(None::<()>);
                    continue;
                }
            };
            if let Some(out) = self.switcher.looper_process(task.on_tick(now)) {
                return Some((index, out));
            }
        }

        None
    }

    /// This function send an event to a task and return the output if the task has output.
    /// If the task has output, we will return the output and save flag last_input_index for next pop_last_input call.
    pub fn on_event(&mut self, now: Instant, index: usize, input: In) -> Option<Out> {
        let task = self.tasks.get_mut(index)?.as_mut()?;
        let out = task.on_event(now, input)?;
        self.switcher.queue_flag_task(index);
        Some(out)
    }

    /// Retrieves the output from the last processed task input event.
    /// In SAN/IO we usually have some output when we receive an input event.
    /// External will call this function util it return None.
    /// We use last_input_index which is saved in previous on_input_event or on_input_tick and clear it after we got None.
    pub fn pop_output(&mut self, now: Instant) -> Option<(usize, Out)> {
        // We dont clear_destroyed_task here because have some case we have output after we received TaskOutput::Destroy the task.
        // We will clear_destroyed_task in next pop_output call.

        while let Some(index) = self.switcher.queue_current() {
            let slot = self.tasks.get_mut(index);
            if let Some(Some(slot)) = slot {
                if let Some(out) = self.switcher.queue_process(slot.pop_output(now)) {
                    return Some((index, out));
                }
            } else {
                self.switcher.queue_process(None::<()>);
            }
        }
        None
    }

    /// Gracefully destroys the task group.
    pub fn shutdown(&mut self, now: Instant) -> Option<(usize, Out)> {
        while let Some(index) = self.switcher.looper_current(now) {
            log::info!("Group kill tasks {}/{}", index, self.switcher.tasks());
            //We only call each task single time
            self.switcher.looper_process(None::<()>);
            let task = match self.tasks.get_mut(index) {
                Some(Some(task)) => task,
                _ => {
                    continue;
                }
            };
            if let Some(out) = task.shutdown(now) {
                return Some((index, out));
            }
        }

        None
    }
}
