use std::collections::VecDeque;

/// A dynamic deque that can store elements in a stack or a heap.
/// First, elements are pushed to the stack. If the stack is full, elements are pushed to the heap.
/// When popping elements, the stack is popped first. If the stack is empty and the heap is not, the
/// heap is popped and the elements are pushed to the stack.
/// Example:
/// ```
/// use sans_io_runtime::DynamicDeque;
/// let mut deque: DynamicDeque<i32, 5> = DynamicDeque::new();
/// deque.push_back(true, 1).unwrap();
/// deque.push_back(true, 2).unwrap();
/// deque.push_back(true, 3).unwrap();
/// deque.push_back(true, 4).unwrap();
/// deque.push_back(true, 5).unwrap();
/// deque.push_back(true, 6).unwrap(); // Should overflow to heap
/// assert_eq!(deque.len(), 6);
/// ```
pub struct DynamicDeque<T, const STACK_SIZE: usize> {
    stack: heapless::Deque<T, STACK_SIZE>,
    heap: VecDeque<T>,
}

#[allow(unused)]
impl<T, const STATIC_SIZE: usize> DynamicDeque<T, STATIC_SIZE> {
    /// Creates a new instance of `DynamicDeque`.
    pub fn new() -> Self {
        Self {
            stack: heapless::Deque::new(),
            heap: VecDeque::new(),
        }
    }

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
        let mut instance = Self::new();
        for item in prepare {
            instance.push_back_safe(item);
        }
        instance
    }

    /// Pushes an element to the back of the deque.
    ///
    /// # Arguments
    ///
    /// * `safe` - A boolean indicating whether this message should be fallback to heap if stack full.
    /// * `value` - The value to be pushed.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If the element was successfully pushed.
    /// * `Err(value)` - If the element could not be pushed due to overflow.
    pub fn push_back(&mut self, safe: bool, value: T) -> Result<(), T> {
        if safe {
            self.push_back_safe(value);
            Ok(())
        } else {
            self.push_back_stack(value)
        }
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
    pub fn push_back_safe(&mut self, value: T) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_back() {
        let mut deque: DynamicDeque<i32, 2> = DynamicDeque::new();
        assert!(deque.push_back(false, 1).is_ok());
        assert!(deque.push_back(false, 2).is_ok());
        assert!(deque.push_back(false, 3).is_err());
        assert!(deque.push_back(true, 3).is_ok());
        assert_eq!(deque.len(), 3);
    }

    #[test]
    fn test_pop_front() {
        let mut deque: DynamicDeque<i32, 2> = DynamicDeque::new();
        deque.push_back(true, 1).unwrap();
        deque.push_back(true, 2).unwrap();
        deque.push_back(true, 3).unwrap();
        assert_eq!(deque.pop_front(), Some(1));
        assert_eq!(deque.pop_front(), Some(2));
        assert_eq!(deque.pop_front(), Some(3));
        assert_eq!(deque.pop_front(), None);
        assert_eq!(deque.len(), 0);
    }

    #[test]
    fn test_is_empty() {
        let mut deque: DynamicDeque<i32, 2> = DynamicDeque::new();
        assert_eq!(deque.is_empty(), true);
        deque.push_back(true, 1).unwrap();
        assert_eq!(deque.is_empty(), false);
        deque.pop_front();
        assert_eq!(deque.is_empty(), true);
    }
}
