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
pub use task::{group::TaskGroup, switcher::TaskSwitcher, Task};
pub use trace::*;
pub use worker::{
    BusChannelControl, BusControl, BusEvent, WorkerInner, WorkerInnerInput, WorkerInnerOutput,
    WorkerStats,
};

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
            fn index(&self) -> usize {
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
