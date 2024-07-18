use std::{marker::PhantomData, ops::Deref};

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
///
/// //we need to pop task index from wait queue
/// switcher.flag_task(0 as usize);
/// assert_eq!(switcher.current(), Some(0));
/// switcher.finished(0 as usize);
/// assert_eq!(switcher.current(), None);
///
/// ```

pub trait TaskSwitcherChild<Out> {
    type Time: Copy;
    fn pop_output(&mut self, now: Self::Time) -> Option<Out>;
}
pub struct TaskSwitcherBranch<Task, Out> {
    task_type: usize,
    task: Task,
    _tmp: PhantomData<Out>,
}

impl<Task: Default, Out> TaskSwitcherBranch<Task, Out> {
    pub fn default<TT: Into<usize>>(tt: TT) -> Self {
        Self {
            task_type: tt.into(),
            task: Default::default(),
            _tmp: Default::default(),
        }
    }
}

impl<Task, Out> TaskSwitcherBranch<Task, Out> {
    pub fn new<TT: Into<usize>>(task: Task, tt: TT) -> Self {
        Self {
            task_type: tt.into(),
            task,
            _tmp: Default::default(),
        }
    }

    pub fn input(&mut self, s: &mut TaskSwitcher) -> &mut Task {
        s.flag_task(self.task_type);
        &mut self.task
    }
}

impl<Task: TaskSwitcherChild<Out>, Out> TaskSwitcherBranch<Task, Out> {
    pub fn pop_output(&mut self, now: Task::Time, s: &mut TaskSwitcher) -> Option<Out> {
        let out = self.task.pop_output(now);
        if out.is_none() {
            s.finished(self.task_type);
        }
        out
    }
}

impl<Task: TaskSwitcherChild<Out>, Out> Deref for TaskSwitcherBranch<Task, Out> {
    type Target = Task;

    fn deref(&self) -> &Self::Target {
        &self.task
    }
}

pub struct TaskSwitcher {
    bits: BitVec,
}

impl TaskSwitcher {
    pub fn new(len: usize) -> Self {
        Self {
            bits: BitVec::news(len),
        }
    }

    pub fn set_tasks(&mut self, tasks: usize) {
        self.bits.set_len(tasks);
    }

    pub fn tasks(&self) -> usize {
        self.bits.len()
    }

    /// Returns the current index of the task group, if it's not finished. Otherwise, returns None.
    pub fn current(&mut self) -> Option<usize> {
        self.bits.first_set_index()
    }

    pub fn flag_all(&mut self) {
        self.bits.set_all(true);
    }

    /// Flag that the current task group is finished.
    pub fn finished<I: Into<usize>>(&mut self, index: I) {
        self.bits.set_bit(index.into(), false);
    }

    pub fn flag_task<I: Into<usize>>(&mut self, index: I) {
        self.bits.set_bit(index.into(), true);
    }
}

#[cfg(test)]
mod tests {
    use crate::TaskSwitcher;

    #[test]
    fn queue_with_stack_like_style() {
        let mut state = TaskSwitcher::new(5);
        state.flag_all();

        assert_eq!(state.current(), Some(0));
        state.finished(0 as usize);
        assert_eq!(state.current(), Some(1));
        state.finished(1 as usize);
        assert_eq!(state.current(), Some(2));
        state.finished(2 as usize);
        assert_eq!(state.current(), Some(3));
        state.finished(3 as usize);
        assert_eq!(state.current(), Some(4));
        state.finished(4 as usize);
        assert_eq!(state.current(), None);

        state.flag_task(3 as usize);

        assert_eq!(state.current(), Some(3));
        state.finished(3 as usize);
        assert_eq!(state.current(), None);
    }

    #[test]
    fn queue_test2() {
        let mut state = TaskSwitcher::new(2);
        state.flag_all();
        assert_eq!(state.current(), Some(0));
        state.finished(0 as usize);
        assert_eq!(state.current(), Some(1));
        state.finished(1 as usize);
        assert_eq!(state.current(), None);

        // next cycle
        state.flag_all();
        assert_eq!(state.current(), Some(0));
        state.finished(0 as usize);
        assert_eq!(state.current(), Some(1));
        state.finished(1 as usize);
        assert_eq!(state.current(), None);
    }

    //TODO test TaskSwitcherChild
}
