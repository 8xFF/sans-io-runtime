//! This module contains the `Owner` type, which represents the owner of a task or group of tasks in the runtime.
//! Example:
//! ```
//! use sans_io_runtime::Owner;
//!
//! let owner = Owner::worker(1);
//! assert_eq!(owner.worker_id(), 1);
//! assert_eq!(owner.group_id(), None);
//! assert_eq!(owner.task_index(), None);
//!
//! let owner = Owner::group(1, 2);
//! assert_eq!(owner.worker_id(), 1);
//! assert_eq!(owner.group_id(), Some(2));
//! assert_eq!(owner.task_index(), None);
//!
//! let owner = Owner::task(1, 2, 3);
//! assert_eq!(owner.worker_id(), 1);
//! assert_eq!(owner.group_id(), Some(2));
//! assert_eq!(owner.task_index(), Some(3));
//! ```

/// Represents the owner of a task or group of tasks in the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    /// Represents a worker.
    Worker { worker: u16 },
    /// Represents a group of tasks.
    Group { worker: u16, group: u16 },
    /// Represents a specific task.
    Task {
        worker: u16,
        group: u16,
        task_index: usize,
    },
}

impl Owner {
    /// Creates a new `Owner` representing a worker.
    pub fn worker(worker: u16) -> Self {
        Self::Worker { worker }
    }

    /// Creates a new `Owner` representing a group of tasks.
    pub fn group(worker: u16, group: u16) -> Self {
        Self::Group { worker, group }
    }

    /// Creates a new `Owner` representing a specific task.
    pub fn task(worker: u16, group: u16, task_index: usize) -> Self {
        Self::Task {
            worker,
            group,
            task_index,
        }
    }

    /// Returns the worker ID associated with the `Owner`.
    pub fn worker_id(&self) -> u16 {
        match self {
            Self::Worker { worker } => *worker,
            Self::Group { worker, .. } => *worker,
            Self::Task { worker, .. } => *worker,
        }
    }

    /// Returns the group ID associated with the `Owner`, if applicable.
    pub fn group_id(&self) -> Option<u16> {
        match self {
            Self::Worker { .. } => None,
            Self::Group { group, .. } => Some(*group),
            Self::Task { group, .. } => Some(*group),
        }
    }

    /// Returns the task index associated with the `Owner`, if applicable.
    pub fn task_index(&self) -> Option<usize> {
        match self {
            Self::Worker { .. } => None,
            Self::Group { .. } => None,
            Self::Task { task_index, .. } => Some(*task_index),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_owner() {
        let owner = Owner::worker(1);
        assert_eq!(owner.worker_id(), 1);
        assert_eq!(owner.group_id(), None);
        assert_eq!(owner.task_index(), None);
    }

    #[test]
    fn test_group_owner() {
        let owner = Owner::group(1, 2);
        assert_eq!(owner.worker_id(), 1);
        assert_eq!(owner.group_id(), Some(2));
        assert_eq!(owner.task_index(), None);
    }

    #[test]
    fn test_task_owner() {
        let owner = Owner::task(1, 2, 3);
        assert_eq!(owner.worker_id(), 1);
        assert_eq!(owner.group_id(), Some(2));
        assert_eq!(owner.task_index(), Some(3));
    }
}
