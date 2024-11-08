use std::{collections::HashSet, fmt::Debug, marker::PhantomData, time::Instant};

use crate::{collections::DynamicVec, Task, TaskSwitcher, TaskSwitcherChild};

#[derive(Debug)]
pub enum TaskGroupOutput<Out> {
    TaskOutput(usize, Out),
    OnResourceEmpty,
}

#[derive(Debug)]
struct TaskContainer<T> {
    task: T,
    is_empty: bool,
}

impl<T> TaskContainer<T> {
    fn new(task: T) -> Self {
        Self {
            task,
            is_empty: false,
        }
    }
}

/// Represents a group of tasks.
#[derive(Debug)]
pub struct TaskGroup<In, Out, T: Task<In, Out>, const STACK_SIZE: usize> {
    tasks: DynamicVec<Option<TaskContainer<T>>, STACK_SIZE>,
    tasks_count: usize,
    empty_tasks: HashSet<usize>,
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
            tasks_count: 0,
            empty_tasks: HashSet::default(),
            _tmp: Default::default(),
            switcher: TaskSwitcher::new(0),
        }
    }
}

impl<In, Out, T: Task<In, Out>, const STACK_SIZE: usize> TaskGroup<In, Out, T, STACK_SIZE> {
    /// Returns the number of tasks in the group.
    pub fn tasks(&self) -> usize {
        self.tasks_count
    }

    /// Check if we have task with index
    pub fn has_task(&self, index: usize) -> bool {
        matches!(self.tasks.get(index), Some(Some(_)))
    }

    /// Adds a task to the group.
    pub fn add_task(&mut self, task: T) -> usize {
        for (index, slot) in self.tasks.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(TaskContainer::new(task));
                self.tasks_count += 1;
                return index;
            }
        }

        self.tasks.push(Some(TaskContainer::new(task)));
        self.switcher.set_tasks(self.tasks.len());
        self.tasks_count += 1;
        self.tasks.len() - 1
    }

    /// Set task to a slot index
    pub fn set_task(&mut self, index: usize, task: T) -> usize {
        while self.tasks.len() <= index {
            self.tasks.push(None);
        }
        self.tasks.set(index, Some(TaskContainer::new(task)));
        self.switcher.set_tasks(self.tasks.len());
        self.tasks_count += 1;
        self.tasks.len() - 1
    }

    /// Remove a task from the group
    pub fn remove_task(&mut self, index: usize) {
        self.tasks.get_mut_or_panic(index).take();
        self.empty_tasks.remove(&index);
        self.tasks_count -= 1;
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
                task.task.on_tick(now);
            }
        }
    }

    /// Send event to correct task with index
    pub fn on_event(&mut self, now: Instant, index: usize, input: In) -> Option<()> {
        let task = self.tasks.get_mut(index)?.as_mut()?;
        self.switcher.flag_task(index);
        task.task.on_event(now, input);
        Some(())
    }

    /// Gracefully destroys the task group.
    pub fn on_shutdown(&mut self, now: Instant) {
        self.switcher.flag_all();
        for index in 0..self.switcher.tasks() {
            log::info!("Group kill tasks {}/{}", index, self.switcher.tasks());
            if let Some(Some(task)) = self.tasks.get_mut(index) {
                task.task.on_shutdown(now);
            }
        }
    }
}

impl<In, Out, T: Task<In, Out>, const STACK_SIZE: usize> TaskSwitcherChild<TaskGroupOutput<Out>>
    for TaskGroup<In, Out, T, STACK_SIZE>
{
    type Time = T::Time;

    fn empty_event(&self) -> TaskGroupOutput<Out> {
        TaskGroupOutput::OnResourceEmpty
    }

    fn is_empty(&self) -> bool {
        self.empty_tasks.len() == self.tasks()
    }

    /// Retrieves the output from the flagged processed task.
    fn pop_output(&mut self, now: Self::Time) -> Option<TaskGroupOutput<Out>> {
        while let Some(index) = self.switcher.current() {
            let slot = self.tasks.get_mut(index);
            if let Some(Some(slot)) = slot {
                if let Some(out) = slot.task.pop_output(now) {
                    return Some(TaskGroupOutput::TaskOutput(index, out));
                } else {
                    if !slot.is_empty {
                        if slot.task.is_empty() {
                            slot.is_empty = true;
                            self.empty_tasks.insert(index);
                            return Some(TaskGroupOutput::TaskOutput(
                                index,
                                slot.task.empty_event(),
                            ));
                        }
                    } else {
                        #[warn(clippy::collapsible_else_if)]
                        if !slot.task.is_empty() {
                            slot.is_empty = false;
                            self.empty_tasks.remove(&index);
                        }
                    }
                    self.switcher.finished(index);
                }
            } else {
                self.switcher.finished(index);
            }
        }
        None
    }
}
