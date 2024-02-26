#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    Worker {
        worker: u16,
    },
    Group {
        worker: u16,
        group: u16,
    },
    Task {
        worker: u16,
        group: u16,
        task_index: usize,
    },
}

impl Owner {
    pub fn worker(worker: u16) -> Self {
        Self::Worker { worker }
    }

    pub fn group(worker: u16, group: u16) -> Self {
        Self::Group { worker, group }
    }

    pub fn task(worker: u16, group: u16, task_index: usize) -> Self {
        Self::Task {
            worker,
            group,
            task_index,
        }
    }

    pub fn worker_id(&self) -> u16 {
        match self {
            Self::Worker { worker } => *worker,
            Self::Group { worker, .. } => *worker,
            Self::Task { worker, .. } => *worker,
        }
    }

    pub fn group_id(&self) -> Option<u16> {
        match self {
            Self::Worker { .. } => None,
            Self::Group { group, .. } => Some(*group),
            Self::Task { group, .. } => Some(*group),
        }
    }

    pub fn task_index(&self) -> usize {
        match self {
            Self::Worker { .. } => 0,
            Self::Group { .. } => 0,
            Self::Task { task_index, .. } => *task_index,
        }
    }
}
