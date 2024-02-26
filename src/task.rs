use std::{net::SocketAddr, time::Instant};

use crate::{owner::Owner, WorkerCtx};

pub enum NetIncoming<'a> {
    UdpListenResult {
        bind: SocketAddr,
        result: Result<SocketAddr, std::io::Error>,
    },
    UdpPacket {
        from: SocketAddr,
        to: SocketAddr,
        data: &'a [u8],
    },
}

pub enum NetOutgoing<'a> {
    UdpListen(SocketAddr),
    UdpPacket {
        from: SocketAddr,
        to: SocketAddr,
        data: &'a [u8],
    },
}

pub enum TaskInput<'a, Ext> {
    Net(NetIncoming<'a>),
    External(Ext),
}

pub enum TaskOutput<'a, Ext> {
    Net(NetOutgoing<'a>),
    External(Ext),
    Destroy,
}

pub trait Task<ExtIn, ExtOut> {
    const TYPE: u16;
    fn on_tick(&mut self, now: Instant);
    fn on_input<'a>(&mut self, now: Instant, input: TaskInput<'a, ExtIn>);
    fn pop_output(&mut self, now: Instant) -> Option<TaskOutput<'_, ExtOut>>;
}

pub enum TaskGroupOutput<'a, Ext> {
    Net(NetOutgoing<'a>),
    External(Ext),
}

pub struct TaskGroup<ExtIn, ExtOut, T: Task<ExtIn, ExtOut>> {
    worker: u16,
    tasks: Vec<T>,
    _tmp: std::marker::PhantomData<(ExtIn, ExtOut)>,
}

impl<ExtIn, ExtOut, T: Task<ExtIn, ExtOut>> TaskGroup<ExtIn, ExtOut, T> {
    pub fn new(worker: u16) -> Self {
        Self {
            worker,
            tasks: vec![],
            _tmp: Default::default(),
        }
    }

    pub fn tasks(&self) -> usize {
        self.tasks.len()
    }

    pub fn add_task(&mut self, task: T) {
        self.tasks.push(task);
    }

    pub fn on_ext(&mut self, _now: Instant, _ctx: &mut WorkerCtx<'_>, _ext: ExtIn) {
        todo!()
    }

    pub fn on_net(&mut self, now: Instant, owner: Owner, net: NetIncoming) {
        self.tasks[owner.task_index()].on_input(now, TaskInput::Net(net));
    }

    pub fn inner_process(&mut self, now: Instant, ctx: &mut WorkerCtx<'_>) {
        let mut destroyed_tasks = vec![];
        for (index, task) in self.tasks.iter_mut().enumerate() {
            task.on_tick(now);
            while let Some(output) = task.pop_output(now) {
                match output {
                    TaskOutput::Net(out) => {
                        ctx.backend
                            .on_action(out, Owner::task(self.worker, T::TYPE, index));
                    }
                    TaskOutput::External(_) => todo!(),
                    TaskOutput::Destroy => {
                        destroyed_tasks.push(index);
                    }
                }
            }
        }
        while let Some(index) = destroyed_tasks.pop() {
            ctx.backend
                .remove_owner(Owner::task(self.worker, T::TYPE, index));
            let last_index = self.tasks.len() - 1;
            if index != last_index {
                self.tasks.swap_remove(index);
                ctx.backend.swap_owner(
                    Owner::task(self.worker, T::TYPE, last_index),
                    Owner::task(self.worker, T::TYPE, index),
                );
                //update element in destroyed_tasks to index if it equals last_index
                for i in destroyed_tasks.iter_mut() {
                    if *i == last_index {
                        *i = index;
                    }
                }
            } else {
                self.tasks.pop();
            }
        }
    }

    pub fn pop_output(&mut self) -> Option<TaskGroupOutput<'_, ExtOut>> {
        None
    }
}
