use std::{marker::PhantomData, time::Instant};

use crate::{collections::DynamicVec, Task, TaskSwitcher, TaskSwitcherChild};

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

    /// Check if we have task with index
    pub fn has_task(&self, index: usize) -> bool {
        matches!(self.tasks.get(index), Some(Some(_)))
    }

    /// Adds a task to the group.
    pub fn add_task(&mut self, task: T) -> usize {
        for (index, slot) in self.tasks.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(task);
                return index;
            }
        }

        self.tasks.push(Some(task));
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

    /// Fire tick event to all tasks, after that we need to call pop_output util it return None
    pub fn on_tick(&mut self, now: Instant) {
        self.switcher.flag_all();
        for index in 0..self.switcher.tasks() {
            if let Some(Some(task)) = self.tasks.get_mut(index) {
                task.on_tick(now);
            }
        }
    }

    /// Send event to correct task with index
    pub fn on_event(&mut self, now: Instant, index: usize, input: In) {
        if let Some(Some(task)) = self.tasks.get_mut(index) {
            self.switcher.flag_task(index);
            task.on_event(now, input);
        }
    }

    /// Gracefully destroys the task group.
    pub fn on_shutdown(&mut self, now: Instant) {
        self.switcher.flag_all();
        for index in 0..self.switcher.tasks() {
            log::info!("Group kill tasks {}/{}", index, self.switcher.tasks());
            if let Some(Some(task)) = self.tasks.get_mut(index) {
                task.on_shutdown(now);
            }
        }
    }
}

impl<In, Out, T: Task<In, Out>, const STACK_SIZE: usize> TaskSwitcherChild<(usize, Out)>
    for TaskGroup<In, Out, T, STACK_SIZE>
{
    type Time = T::Time;

    /// Retrieves the output from the flagged processed task.
    fn pop_output(&mut self, now: Self::Time) -> Option<(usize, Out)> {
        while let Some(index) = self.switcher.current() {
            let slot = self.tasks.get_mut(index);
            if let Some(Some(slot)) = slot {
                if let Some(out) = slot.pop_output(now) {
                    return Some((index, out));
                } else {
                    self.switcher.finished(index);
                }
            } else {
                self.switcher.finished(index);
            }
        }
        None
    }
}
