use std::time::Instant;

pub mod group;
pub mod switcher;

/// Represents a task.
pub trait Task<In, Out> {
    /// Called each time the task is ticked. Default is 1ms.
    fn on_tick(&mut self, now: Instant) -> Option<Out>;

    /// Called when an input event is received for the task.
    fn on_event(&mut self, now: Instant, input: In) -> Option<Out>;

    /// Retrieves the next output event from the task.
    fn pop_output(&mut self, now: Instant) -> Option<Out>;

    /// Gracefully shuts down the task.
    fn shutdown(&mut self, now: Instant) -> Option<Out>;
}
