use std::time::Instant;

use crate::collections::BitVec;

/// Task group outputs state
/// This is used for jumping between multi groups of tasks
/// For example, we have two group G1 and G2,
/// Each cycle we need to call G1.output until it returns None, then we call G2.output until it returns None, then we will have this cycle is finished
/// Then we will start a new cycle and it loop like that
///
/// ```rust
/// use std::time::Instant;
/// use sans_io_runtime::TaskSwitcher;
///
/// // This will create a task switcher with 2 tasks, use 2 bytes, we can adjust max to 16 tasks
/// let mut switcher = TaskSwitcher::new(2);
///
/// let time1 = Instant::now();
/// assert_eq!(switcher.queue_current(), None);
/// assert_eq!(switcher.looper_current(time1), Some(0));
/// switcher.looper_process(Some(1));
///
/// //now we need to pop queue
/// assert_eq!(switcher.queue_current(), Some(0));
/// switcher.queue_process(None::<u8>);
/// assert_eq!(switcher.queue_current(), None);
///
/// //next we need to continue loop
/// assert_eq!(switcher.looper_current(time1), Some(0));
/// switcher.looper_process(None::<u8>);
/// assert_eq!(switcher.looper_current(time1), Some(1));
/// switcher.looper_process(None::<u8>);
/// assert_eq!(switcher.looper_current(time1), None);
/// assert_eq!(switcher.queue_current(), None);
/// ```
struct TaskQueue {
    bits: BitVec,
    last_index: Option<usize>,
}

impl TaskQueue {
    pub fn new(len: usize) -> Self {
        Self {
            bits: BitVec::news(len),
            last_index: None,
        }
    }

    pub fn set_tasks(&mut self, tasks: usize) {
        self.bits.set_len(tasks);
    }

    /// Returns the current index of the task group, if it's not finished. Otherwise, returns None.
    pub fn current(&mut self) -> Option<usize> {
        let res = self.bits.first_set_index()?;
        self.last_index = Some(res);
        Some(res)
    }

    pub fn flag_all(&mut self) {
        self.bits.set_all(true);
    }

    /// Flag that the current task group is finished.
    pub fn process<R>(&mut self, res: Option<R>) -> Option<R> {
        if res.is_none() {
            if let Some(last) = self.last_index {
                self.last_index = None;
                self.bits.set_bit(last, false);
            }
        }
        res
    }

    pub fn flag_task(&mut self, index: usize) {
        self.bits.set_bit(index, true);
    }
}

struct TaskLooper {
    tasks: usize,
    last_ts: Option<Instant>,
    current: Option<usize>,
}

impl TaskLooper {
    pub fn new(tasks: usize) -> Self {
        Self {
            tasks,
            last_ts: None,
            current: None,
        }
    }

    pub fn set_tasks(&mut self, tasks: usize) {
        self.tasks = tasks;
        //clear current task if it's out of range
        if let Some(current) = self.current {
            if current >= tasks {
                self.current = None;
            }
        }
    }

    pub fn current(&mut self, now: Instant) -> Option<usize> {
        if self.last_ts != Some(now) && self.tasks > 0 {
            self.current = Some(0);
            self.last_ts = Some(now);
        }
        self.current
    }

    pub fn process<R>(&mut self, res: Option<R>) -> Option<R> {
        if res.is_none() {
            let current = self.current.expect("Should have current");
            if current + 1 < self.tasks {
                self.current = Some(current + 1);
            } else {
                self.current = None;
            }
        }
        res
    }
}

pub struct TaskSwitcher {
    looper: TaskLooper,
    queue: TaskQueue,
}

impl TaskSwitcher {
    pub fn new(tasks: usize) -> Self {
        Self {
            looper: TaskLooper::new(tasks),
            queue: TaskQueue::new(tasks),
        }
    }

    pub fn tasks(&self) -> usize {
        self.looper.tasks
    }

    pub fn set_tasks(&mut self, tasks: usize) {
        self.looper.set_tasks(tasks);
        self.queue.set_tasks(tasks);
    }

    pub fn queue_flag_task(&mut self, index: usize) {
        self.queue.flag_task(index);
    }

    pub fn looper_current(&mut self, now: Instant) -> Option<usize> {
        self.looper.current(now)
    }

    pub fn queue_current(&mut self) -> Option<usize> {
        self.queue.current()
    }

    pub fn looper_process<R>(&mut self, res: Option<R>) -> Option<R> {
        if res.is_some() {
            if let Some(loop_index) = self.looper.current {
                self.queue.flag_task(loop_index);
            }
        }
        self.looper.process(res)
    }

    pub fn queue_process<R>(&mut self, res: Option<R>) -> Option<R> {
        self.queue.process(res)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::TaskSwitcher;

    use super::{TaskLooper, TaskQueue};

    #[test]
    fn queue_with_stack_like_style() {
        let mut state = TaskQueue::new(5);
        state.flag_all();

        assert_eq!(state.current(), Some(0));
        state.process(Some(1));
        assert_eq!(state.current(), Some(0));
        state.process(None::<u8>);
        assert_eq!(state.current(), Some(1));
        state.process(Some(1));
        assert_eq!(state.current(), Some(1));
        state.process(None::<u8>);
        assert_eq!(state.current(), Some(2));
        state.process(None::<u8>);
        assert_eq!(state.current(), Some(3));
        state.process(None::<u8>);
        assert_eq!(state.current(), Some(4));
        state.process(None::<u8>);
        assert_eq!(state.current(), None);

        state.flag_task(3);

        assert_eq!(state.current(), Some(3));
        state.process(Some(1));
        assert_eq!(state.current(), Some(3));
        state.process(None::<u8>);
        assert_eq!(state.current(), None);
    }

    #[test]
    fn queue_test2() {
        let mut state = TaskQueue::new(2);
        state.flag_all();
        assert_eq!(state.current(), Some(0));
        state.process(Some(1));
        assert_eq!(state.current(), Some(0));
        state.process(None::<u8>);
        assert_eq!(state.current(), Some(1));
        state.process(None::<u8>);
        assert_eq!(state.current(), None);

        // next cycle
        state.flag_all();
        assert_eq!(state.current(), Some(0));
        state.process(None::<u8>);
        assert_eq!(state.current(), Some(1));
        state.process(None::<u8>);
        assert_eq!(state.current(), None);
    }

    #[test]
    fn looper_with_memory_style() {
        let mut looper = TaskLooper::new(3);
        let current = Instant::now();

        assert_eq!(looper.current(current), Some(0));
        looper.process(None::<u8>);
        assert_eq!(looper.current(current), Some(1));
        looper.process(Some(1));
        assert_eq!(looper.current(current), Some(1));
        looper.process(None::<u8>);
        assert_eq!(looper.current(current), Some(2));
        looper.process(None::<u8>);
        assert_eq!(looper.current(current), None);

        std::thread::sleep(Duration::from_millis(50));
        let next = Instant::now();

        assert_eq!(looper.current(next), Some(0));
        looper.process(Some(1));
        assert_eq!(looper.current(next), Some(0));
        looper.process(None::<u8>);
        assert_eq!(looper.current(next), Some(1));
        looper.process(None::<u8>);
        assert_eq!(looper.current(next), Some(2));
        looper.process(None::<u8>);
        assert_eq!(looper.current(next), None);
    }

    /// We need to clear current task if it's out of range
    #[test]
    fn looper_set_tasks_clear_current() {
        let mut looper = TaskLooper::new(1);
        let current = Instant::now();

        assert_eq!(looper.current(current), Some(0));
        looper.process(Some(()));

        looper.set_tasks(0);
        assert_eq!(looper.current(current), None);
    }

    #[test]
    fn switcher_complex() {
        let mut switcher = TaskSwitcher::new(2);

        let time1 = Instant::now();
        assert_eq!(switcher.queue_current(), None);
        assert_eq!(switcher.looper_current(time1), Some(0));
        switcher.looper_process(Some(1));

        //now we need to pop queue
        assert_eq!(switcher.queue_current(), Some(0));
        switcher.queue_process(None::<u8>);
        assert_eq!(switcher.queue_current(), None);

        //next we need to continue loop
        assert_eq!(switcher.looper_current(time1), Some(0));
        switcher.looper_process(None::<u8>);
        assert_eq!(switcher.looper_current(time1), Some(1));
        switcher.looper_process(None::<u8>);
        assert_eq!(switcher.looper_current(time1), None);
        assert_eq!(switcher.queue_current(), None);
    }
}
