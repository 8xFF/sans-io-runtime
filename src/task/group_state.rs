/// Task group outputs state
/// This is used for jumping between multi groups of tasks
/// For example, we have two group G1 and G2,
/// Each cycle we need to call G1.output until it returns None, then we call G2.output until it returns None, then we will have this cycle is finished
/// Then we will start a new cycle and it loop like that
///
/// ```rust
/// use sans_io_runtime::TaskGroupOutputsState;
///
/// let mut state = TaskGroupOutputsState::<2>::default();
/// assert_eq!(state.current(), Some(0));
/// state.process(Some(1));
/// assert_eq!(state.current(), Some(0));
/// state.process(None::<u8>);
/// assert_eq!(state.current(), Some(1));
/// state.process(None::<u8>);
/// assert_eq!(state.current(), None);
///
/// // next cycle
/// assert_eq!(state.current(), Some(0));
/// state.process(None::<u8>);
/// assert_eq!(state.current(), Some(1));
/// state.process(None::<u8>);
/// assert_eq!(state.current(), None);
/// ```
#[derive(Default)]
pub struct TaskGroupOutputsState<const LEN: u16> {
    current_index: u16,
}

impl<const LEN: u16> TaskGroupOutputsState<LEN> {
    /// Returns the current index of the task group, if it's not finished. Otherwise, returns None.
    pub fn current(&mut self) -> Option<u16> {
        if self.current_index < LEN {
            Some(self.current_index)
        } else {
            self.current_index = 0;
            None
        }
    }

    /// Flag that the current task group is finished.
    pub fn process<R>(&mut self, res: Option<R>) -> Option<R> {
        if res.is_none() {
            self.current_index += 1;
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::TaskGroupOutputsState;

    #[test]
    fn test_group_outputs() {
        let mut state = TaskGroupOutputsState::<2>::default();
        assert_eq!(state.current(), Some(0));
        state.process(Some(1));
        assert_eq!(state.current(), Some(0));
        state.process(None::<u8>);
        assert_eq!(state.current(), Some(1));
        state.process(None::<u8>);
        assert_eq!(state.current(), None);

        // next cycle
        assert_eq!(state.current(), Some(0));
        state.process(None::<u8>);
        assert_eq!(state.current(), Some(1));
        state.process(None::<u8>);
        assert_eq!(state.current(), None);
    }
}
