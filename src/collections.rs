use std::collections::VecDeque;

pub struct DynamicDeque<T, const STACK_SIZE: usize> {
    stack: heapless::Deque<T, STACK_SIZE>,
    heap: VecDeque<T>,
}

impl<T, const STATIC_SIZE: usize> DynamicDeque<T, STATIC_SIZE> {
    pub fn new() -> Self {
        Self {
            stack: heapless::Deque::new(),
            heap: VecDeque::new(),
        }
    }

    pub fn from<const SIZE: usize>(prepare: [T; SIZE]) -> Self {
        let mut instance = Self::new();
        for item in prepare {
            instance.push_back_safe(item);
        }
        instance
    }

    pub fn push_back(&mut self, safe: bool, value: T) -> Result<(), T> {
        if safe {
            self.push_back_safe(value);
            Ok(())
        } else {
            self.push_back_stack(value)
        }
    }

    pub fn push_back_stack(&mut self, value: T) -> Result<(), T> {
        self.stack.push_back(value)
    }

    pub fn push_back_safe(&mut self, value: T) {
        if let Err(value) = self.stack.push_back(value) {
            self.heap.push_back(value);
        }
    }

    pub fn pop_front(&mut self) -> Option<T> {
        let res = self.stack.pop_front();
        if !self.stack.is_full() && !self.heap.is_empty() {
            let front = self.heap.pop_front().expect("Should have");
            if self.stack.push_back(front).is_err() {
                panic!("cannot happend");
            }
        }
        res
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn len(&self) -> usize {
        self.stack.len() + self.heap.len()
    }
}

pub struct DynamicVec<T, const STACK_SIZE: usize> {
    stack: heapless::Vec<T, STACK_SIZE>,
    heap: Vec<T>,
}

impl<T, const STACK_SIZE: usize> DynamicVec<T, STACK_SIZE> {
    pub fn new() -> Self {
        Self {
            stack: heapless::Vec::new(),
            heap: Vec::new(),
        }
    }

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

    pub fn get(&self, index: usize) -> Option<&T> {
        if index < self.stack.len() {
            Some(&self.stack[index])
        } else {
            self.heap.get(index - self.stack.len())
        }
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index < self.stack.len() {
            Some(&mut self.stack[index])
        } else {
            self.heap.get_mut(index - self.stack.len())
        }
    }

    //overwrite []
    pub fn get_mut_or_panic(&mut self, index: usize) -> &mut T {
        if index < self.stack.len() {
            &mut self.stack[index]
        } else {
            &mut self.heap[index - self.stack.len()]
        }
    }

    pub fn push(&mut self, safe: bool, value: T) -> Result<(), T> {
        if safe {
            self.push_safe(value);
            Ok(())
        } else {
            self.push_stack(value)
        }
    }

    pub fn push_stack(&mut self, value: T) -> Result<(), T> {
        self.stack.push(value)
    }

    pub fn push_safe(&mut self, value: T) {
        if let Err(value) = self.stack.push(value) {
            self.heap.push(value);
        }
    }

    pub fn pop(&mut self) -> Option<T> {
        if let Some(t) = self.heap.pop() {
            Some(t)
        } else {
            self.stack.pop()
        }
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn len(&self) -> usize {
        self.stack.len() + self.heap.len()
    }
}
