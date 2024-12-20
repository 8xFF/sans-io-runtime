pub mod backend;
mod buffer;
pub mod bus;
pub mod collections;
mod controller;
mod task;
mod trace;
mod worker;

pub use buffer::*;
pub use controller::Controller;
pub use task::{
    group::{TaskGroup, TaskGroupOutput},
    switcher::{TaskSwitcher, TaskSwitcherBranch, TaskSwitcherChild},
    Task,
};
pub use trace::*;
pub use worker::{
    BusChannelControl, BusControl, BusEvent, WorkerInner, WorkerInnerInput, WorkerInnerOutput,
    WorkerStats,
};

#[macro_export]
macro_rules! return_if_none {
    ($option:expr) => {
        match $option {
            Some(val) => val,
            None => return,
        }
    };
}

#[macro_export]
macro_rules! return_if_some {
    ($option:expr) => {
        let out = $option;
        if out.is_some() {
            return out;
        }
    };
}

#[macro_export]
macro_rules! return_if_err {
    ($option:expr) => {
        match $option {
            Ok(val) => val,
            Err(_) => return,
        }
    };
}

#[macro_export]
macro_rules! group_owner_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
        pub struct $name(usize);

        #[allow(dead_code)]
        impl $name {
            fn build(index: usize) -> Self {
                Self(index)
            }
            pub fn index(&self) -> usize {
                self.0
            }
        }

        impl From<usize> for $name {
            fn from(value: usize) -> Self {
                Self(value)
            }
        }
    };
}

#[macro_export]
macro_rules! group_task {
    ($name:ident, $task:ty, $input:ty, $output:ty) => {
        pub struct $name {
            tasks: Vec<Option<$task>>,
            switcher: TaskSwitcher,
        }

        impl Default for $name {
            fn default() -> Self {
                Self {
                    tasks: Vec::new(),
                    switcher: TaskSwitcher::new(0),
                }
            }
        }

        impl $name {
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
            pub fn add_task(&mut self, task: $task) -> usize {
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
                self.tasks
                    .get_mut(index)
                    .expect("Should have task when remove")
                    .take();
                while let Some(None) = self.tasks.last() {
                    self.tasks.pop();
                }
                self.switcher.set_tasks(self.tasks.len());
            }

            pub fn on_tick(&mut self, now: std::time::Instant) {
                self.switcher.flag_all();
                for index in 0..self.switcher.tasks() {
                    if let Some(Some(task)) = self.tasks.get_mut(index) {
                        task.on_tick(now);
                    }
                }
            }

            pub fn on_event<'a>(
                &mut self,
                now: std::time::Instant,
                index: usize,
                input: $input,
            ) -> Option<()> {
                let task = self.tasks.get_mut(index)?.as_mut()?;
                self.switcher.flag_task(index);
                task.on_event(now, input);
                Some(())
            }

            pub fn pop_output<'a>(&mut self, now: std::time::Instant) -> Option<(usize, $output)> {
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

            /// Gracefully destroys the task group.
            pub fn on_shutdown(&mut self, now: std::time::Instant) {
                self.switcher.flag_all();
                for index in 0..self.switcher.tasks() {
                    log::info!("Group kill tasks {}/{}", index, self.switcher.tasks());
                    if let Some(Some(task)) = self.tasks.get_mut(index) {
                        task.on_shutdown(now);
                    }
                }
            }
        }
    };
}
