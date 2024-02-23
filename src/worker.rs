use std::time::{Duration, Instant};

use crate::{
    backend::Backend,
    bus::{BusLegEvent, BusLegReceiver, BusLocalHub, BusMultiDest, BusSingleDest, BusSystemLocal},
    task::Task,
    BusEvent, Input, Output,
};

#[derive(Debug, Clone)]
pub enum WorkerControlIn<TCfg> {
    Spawn(TCfg),
}

#[derive(Debug)]
pub enum WorkerControlOut {
    Stats(WorkerStats),
}

#[derive(Debug, Default)]
pub struct WorkerStats {
    pub tasks: usize,
    pub ultilization: u32,
}

impl WorkerStats {
    pub fn load(&self) -> u32 {
        self.tasks as u32
    }
}

pub struct Worker<
    ExtIn,
    ExtOut,
    MSG: Send + Sync + Clone,
    T: Task<ExtIn, ExtOut, MSG, TCfg>,
    TCfg,
    B: Backend<usize>,
> {
    tasks: Vec<T>,
    backend: B,
    worker_out_bus: BusSystemLocal<WorkerControlOut>,
    control_recv: BusLegReceiver<WorkerControlIn<TCfg>>,
    internal_bus: BusSystemLocal<MSG>,
    internal_recv: BusLegReceiver<MSG>,
    bus_local_hub: BusLocalHub<usize>,
    _tmp: std::marker::PhantomData<(ExtIn, ExtOut)>,
}

impl<
        ExtIn,
        ExtOut,
        MSG: Send + Sync + Clone,
        T: Task<ExtIn, ExtOut, MSG, TCfg>,
        B: Backend<usize>,
        TCfg,
    > Worker<ExtIn, ExtOut, MSG, T, TCfg, B>
{
    pub fn new(
        worker_out_bus: BusSystemLocal<WorkerControlOut>,
        control_recv: BusLegReceiver<WorkerControlIn<TCfg>>,
        internal_bus: BusSystemLocal<MSG>,
        internal_recv: BusLegReceiver<MSG>,
    ) -> Self {
        Self {
            tasks: Vec::new(),
            backend: Default::default(),
            worker_out_bus,
            control_recv,
            internal_bus,
            internal_recv,
            bus_local_hub: Default::default(),
            _tmp: Default::default(),
        }
    }

    pub fn process(&mut self) {
        let now = Instant::now();
        while let Some(control) = self.control_recv.recv() {
            match control {
                BusLegEvent::Direct(_source_leg, WorkerControlIn::Spawn(cfg)) => {
                    let task = T::build(cfg);
                    self.tasks.push(task);
                }
                _ => {
                    unreachable!("WorkerControlIn only has Spawn variant");
                }
            }
        }

        while let Some(msg) = self.internal_recv.recv() {
            match msg {
                BusLegEvent::Channel(_source_legs, channel, msg) => {
                    if let Some(tasks) = self.bus_local_hub.get_subscribers(channel) {
                        for task in tasks {
                            self.tasks[*task].on_input(now, Input::Bus(msg.clone()));
                        }
                    }
                }
                BusLegEvent::Broadcast(_, _) => todo!(),
                BusLegEvent::Direct(_, _) => todo!(),
            }
        }

        let mut destroyed_tasks = vec![];
        for (index, task) in self.tasks.iter_mut().enumerate() {
            task.on_tick(now);
            while let Some(output) = task.pop_output(now) {
                match output {
                    Output::Net(out) => {
                        self.backend.on_action(out, index);
                    }
                    Output::Bus(BusEvent::ChannelSubscribe(channel)) => {
                        if self.bus_local_hub.subscribe(channel, index) {
                            log::info!("Worker {} subscribe to channel {}", index, *channel);
                            self.internal_bus.subscribe(channel);
                        }
                    }
                    Output::Bus(BusEvent::ChannelUnsubscribe(channel)) => {
                        if self.bus_local_hub.unsubscribe(channel, index) {
                            log::info!("Worker {} unsubscribe from channel {}", index, *channel);
                            self.internal_bus.unsubscribe(channel);
                        }
                    }
                    Output::Bus(BusEvent::ChannelPublish(channel, msg)) => {
                        self.internal_bus.publish(channel, msg);
                    }
                    Output::Bus(BusEvent::Broadcast(_)) => todo!(),
                    Output::Bus(BusEvent::Direct(_, _)) => todo!(),
                    Output::External(_) => todo!(),
                    Output::Destroy => {
                        destroyed_tasks.push(index);
                    }
                }
            }
        }
        for index in destroyed_tasks {
            let last_index = self.tasks.len() - 1;
            self.backend.remove_owner(index);
            self.bus_local_hub.remove_owner(index);
            self.tasks.swap_remove(index);
            self.bus_local_hub.swap_owner(last_index, index);
            self.backend.swap_owner(last_index, index);
        }

        self.backend.finish_outgoing_cycle();

        while let Some((event, task_index)) = self.backend.pop_incoming(Duration::from_millis(1)) {
            self.tasks[task_index].on_input(now, Input::Net(event));
        }

        self.backend.finish_incoming_cycle();

        if let Err(e) = self.worker_out_bus.send(
            0,
            WorkerControlOut::Stats(WorkerStats {
                tasks: self.tasks.len(),
                ultilization: 0,
            }),
        ) {
            log::error!("Failed to send stats: {:?}", e);
        }
    }
}
