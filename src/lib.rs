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
    group::{TaskGroup, TaskGroupInput, TaskGroupOutput, TaskGroupOwner},
    switcher::TaskSwitcher,
    NetIncoming, NetOutgoing, Task, TaskInput, TaskOutput,
};
pub use trace::*;
pub use worker::{WorkerInner, WorkerInnerInput, WorkerInnerOutput, WorkerStats};

#[macro_export]
macro_rules! group_owner_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
        pub struct $name(usize);

        impl TaskGroupOwner for $name {
            fn build(index: usize) -> Self {
                Self(index)
            }
            fn task_index(&self) -> usize {
                self.0
            }
        }
    };
}
