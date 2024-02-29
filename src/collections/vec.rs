/// A dynamic vector that can store elements on both the stack and the heap.
///
/// The `DynamicVec` struct provides a flexible vector implementation that can store elements
/// on the stack using the `heapless::Vec` type and on the heap using the `Vec` type.
/// It allows for efficient storage of small vectors on the stack while automatically
/// falling back to the heap for larger vectors.
///
/// # Examples
///
/// Creating a new `DynamicVec`:
///
/// ```
/// use sans_io_runtime::collections::DynamicVec;
///
/// let mut vec: DynamicVec<u32, 10> = DynamicVec::new();
/// ```
///
/// Pushing elements into the `DynamicVec`:
///
/// ```
/// use sans_io_runtime::collections::DynamicVec;
///
/// let mut vec: DynamicVec<u32, 10> = DynamicVec::new();
/// vec.push(true, 1).unwrap();
/// vec.push(true, 2).unwrap();
/// vec.push(false, 3).unwrap();
/// ```
///
/// Accessing elements in the `DynamicVec`:
///
/// ```
/// use sans_io_runtime::collections::DynamicVec;
///
/// let mut vec: DynamicVec<u32, 10> = DynamicVec::new();
/// vec.push(true, 1).unwrap();
/// vec.push(true, 2).unwrap();
/// vec.push(false, 3).unwrap();
///
/// assert_eq!(vec.get(0), Some(&1));
/// assert_eq!(vec.get(1), Some(&2));
/// assert_eq!(vec.get(2), Some(&3));
/// ```
pub struct DynamicVec<T, const STACK_SIZE: usize> {
    stack: heapless::Vec<T, STACK_SIZE>,
    heap: Vec<T>,
}

#[allow(unused)]
impl<T, const STACK_SIZE: usize> DynamicVec<T, STACK_SIZE> {
    /// Creates a new instance of `DynamicVec`.
    pub fn new() -> Self {
        Self {
            stack: heapless::Vec::new(),
            heap: Vec::new(),
        }
    }

    /// Creates a new instance of `DynamicVec` from an array of elements.
    pub fn from<const SIZE: usize>(prepare: [T; SIZE]) -> Self {
        let mut instance = Self::new();
        for item in prepare {
            instance.push_safe(item);
        }
        instance
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.stack.iter_mut().chain(self.heap.iter_mut())
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.stack.iter().chain(self.heap.iter())
    }

    /// Get the element at the given index.
    pub fn get(&self, index: usize) -> Option<&T> {
        if index < self.stack.len() {
            Some(&self.stack[index])
        } else {
            self.heap.get(index - self.stack.len())
        }
    }

    /// Get the mutable element at the given index.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index < self.stack.len() {
            Some(&mut self.stack[index])
        } else {
            self.heap.get_mut(index - self.stack.len())
        }
    }

    /// Get the mutable element at the given index or panic if not exists.
    pub fn get_mut_or_panic(&mut self, index: usize) -> &mut T {
        if index < self.stack.len() {
            &mut self.stack[index]
        } else {
            &mut self.heap[index - self.stack.len()]
        }
    }

    /// Pushes an element to the vector. If safe is true, it will fallback to heap if stack full.
    pub fn push(&mut self, safe: bool, value: T) -> Result<(), T> {
        if safe {
            self.push_safe(value);
            Ok(())
        } else {
            self.push_stack(value)
        }
    }

    /// Push an element to the stack of the vector.
    pub fn push_stack(&mut self, value: T) -> Result<(), T> {
        self.stack.push(value)
    }

    /// Push an element to the stack or the heap of the vector.
    pub fn push_safe(&mut self, value: T) {
        if let Err(value) = self.stack.push(value) {
            self.heap.push(value);
        }
    }

    /// Pops the element from the end of the vector.
    pub fn pop(&mut self) -> Option<T> {
        if let Some(t) = self.heap.pop() {
            Some(t)
        } else {
            self.stack.pop()
        }
    }

    /// Check if the vector is empty.
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Returns the number of elements in the vector.
    pub fn len(&self) -> usize {
        self.stack.len() + self.heap.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let vec: DynamicVec<u32, 10> = DynamicVec::new();
        assert!(vec.is_empty());
        assert_eq!(vec.len(), 0);
    }

    #[test]
    fn test_from() {
        let arr = [1, 2, 3, 4, 5];
        let vec: DynamicVec<u32, 10> = DynamicVec::from(arr);
        assert_eq!(vec.len(), arr.len());
        for (i, item) in arr.iter().enumerate() {
            assert_eq!(vec.get(i), Some(item));
        }
    }

    #[test]
    fn test_push() {
        let mut vec: DynamicVec<u32, 2> = DynamicVec::new();
        assert_eq!(vec.push(true, 1), Ok(()));
        assert_eq!(vec.push(true, 2), Ok(()));
        assert_eq!(vec.push(true, 3), Ok(()));
        assert_eq!(vec.push(false, 4), Err(4));
        assert_eq!(vec.len(), 3);
        assert_eq!(vec.get(0), Some(&1));
        assert_eq!(vec.get(1), Some(&2));
        assert_eq!(vec.get(2), Some(&3));
    }

    #[test]
    fn test_push_stack() {
        let mut vec: DynamicVec<u32, 2> = DynamicVec::new();
        assert_eq!(vec.push_stack(1), Ok(()));
        assert_eq!(vec.push_stack(2), Ok(()));
        assert_eq!(vec.push_stack(3), Err(3));
        assert_eq!(vec.len(), 2);
        assert_eq!(vec.get(0), Some(&1));
        assert_eq!(vec.get(1), Some(&2));
    }

    #[test]
    fn test_push_safe() {
        let mut vec: DynamicVec<u32, 2> = DynamicVec::new();
        vec.push_safe(1);
        vec.push_safe(2);
        vec.push_safe(3);
        assert_eq!(vec.len(), 3);
        assert_eq!(vec.get(0), Some(&1));
        assert_eq!(vec.get_mut_or_panic(0), &1);
        assert_eq!(vec.get(1), Some(&2));
        assert_eq!(vec.get_mut_or_panic(1), &2);
        assert_eq!(vec.get(2), Some(&3));
        assert_eq!(vec.get_mut_or_panic(2), &3);
    }

    #[test]
    fn test_pop() {
        let mut vec: DynamicVec<u32, 2> = DynamicVec::new();
        vec.push_safe(1);
        vec.push_safe(2);
        assert_eq!(vec.pop(), Some(2));
        assert_eq!(vec.pop(), Some(1));
        assert_eq!(vec.pop(), None);
        assert_eq!(vec.len(), 0);
        assert!(vec.is_empty());
    }
}
