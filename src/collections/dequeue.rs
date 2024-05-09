use std::collections::VecDeque;

/// A dynamic deque that can store elements in a stack or a heap.
/// First, elements are pushed to the stack. If the stack is full, elements are pushed to the heap.
/// When popping elements, the stack is popped first. If the stack is empty and the heap is not, the
/// heap is popped and the elements are pushed to the stack.
/// Example:
/// ```
/// use sans_io_runtime::collections::DynamicDeque;
/// let mut deque: DynamicDeque<i32, 5> = DynamicDeque::default();
/// deque.push_back_stack(1).unwrap();
/// deque.push_back_stack(2).unwrap();
/// deque.push_back_stack(3).unwrap();
/// deque.push_back_stack(4).unwrap();
/// deque.push_back_stack(5).unwrap();
/// deque.push_back(6); // Should overflow to heap
/// assert_eq!(deque.len(), 6);
/// ```
#[derive(Debug)]
pub struct DynamicDeque<T, const STACK_SIZE: usize> {
    stack: heapless::Deque<T, STACK_SIZE>,
    heap: VecDeque<T>,
}

impl<T, const STACK_SIZE: usize> Default for DynamicDeque<T, STACK_SIZE> {
    fn default() -> Self {
        Self {
            stack: heapless::Deque::new(),
            heap: VecDeque::new(),
        }
    }
}

#[allow(unused)]
impl<T, const STATIC_SIZE: usize> DynamicDeque<T, STATIC_SIZE> {
    /// Creates a new instance of `DynamicDeque` from an array of elements.
    ///
    /// # Arguments
    ///
    /// * `prepare` - An array of elements to initialize the deque with.
    ///
    /// # Returns
    ///
    /// A new instance of `DynamicDeque`.
    pub fn from<const SIZE: usize>(prepare: [T; SIZE]) -> Self {
        let mut instance = Self::default();
        for item in prepare {
            instance.push_back(item);
        }
        instance
    }

    /// Pushes an element to the stack of the deque.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to be pushed.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If the element was successfully pushed.
    /// * `Err(value)` - If the element could not be pushed due to overflow.
    pub fn push_back_stack(&mut self, value: T) -> Result<(), T> {
        self.stack.push_back(value)
    }

    /// Pushes an element to the stack or the heap of the deque.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to be pushed.
    pub fn push_back(&mut self, value: T) {
        if let Err(value) = self.stack.push_back(value) {
            self.heap.push_back(value);
        }
    }

    /// Pops the element from the front of the deque.
    ///
    /// # Returns
    ///
    /// The popped element, or `None` if the deque is empty.
    pub fn pop_front(&mut self) -> Option<T> {
        let res = self.stack.pop_front();
        if !self.stack.is_full() && !self.heap.is_empty() {
            let front = self.heap.pop_front().expect("Should have");
            if self.stack.push_back(front).is_err() {
                panic!("cannot happen");
            }
        }
        res
    }

    /// Checks if the deque is empty.
    ///
    /// # Returns
    ///
    /// `true` if the deque is empty, `false` otherwise.
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Returns the number of elements in the deque.
    ///
    /// # Returns
    ///
    /// The number of elements in the deque.
    pub fn len(&self) -> usize {
        self.stack.len() + self.heap.len()
    }

    /// This is useful in task where we usually doing output in queue
    /// We need push to back then immediate pop from front
    #[inline(always)]
    pub fn pop2(&mut self, e: T) -> T {
        if self.is_empty() {
            e
        } else {
            let out = self.pop_front().expect("Should have because just push");
            self.push_back(e);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_back() {
        let mut deque: DynamicDeque<i32, 2> = DynamicDeque::default();
        assert!(deque.push_back_stack(1).is_ok());
        assert!(deque.push_back_stack(2).is_ok());
        assert!(deque.push_back_stack(3).is_err());
        assert_eq!(deque.len(), 2);
        deque.push_back(3);
        assert_eq!(deque.len(), 3);
    }

    #[test]
    fn test_pop_front() {
        let mut deque: DynamicDeque<i32, 2> = DynamicDeque::default();
        deque.push_back(1);
        deque.push_back(2);
        deque.push_back(3);
        assert_eq!(deque.pop_front(), Some(1));
        assert_eq!(deque.pop_front(), Some(2));
        assert_eq!(deque.pop_front(), Some(3));
        assert_eq!(deque.pop_front(), None);
        assert_eq!(deque.len(), 0);
    }

    #[test]
    fn test_is_empty() {
        let mut deque: DynamicDeque<i32, 2> = DynamicDeque::default();
        assert_eq!(deque.is_empty(), true);
        deque.push_back(1);
        assert_eq!(deque.is_empty(), false);
        deque.pop_front();
        assert_eq!(deque.is_empty(), true);
    }
}
