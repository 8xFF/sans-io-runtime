use std::collections::VecDeque;

use crate::{
    backend::Backend,
    bus::{BusLegEvent, BusLegReceiver, BusSingleDest, BusSystemBuilder, BusSystemLocal},
    task::Task,
    worker::{self, WorkerControlIn, WorkerControlOut, WorkerStats},
};

struct WorkerContainer {
    _join: std::thread::JoinHandle<()>,
    stats: WorkerStats,
}

pub struct Controller<
    ExtIn,
    ExtOut,
    MSG: 'static + Clone + Send + Sync,
    T: Task<ExtIn, ExtOut, MSG, TCfg>,
    TCfg,
    B: Backend<usize>,
> {
    workers: usize,
    worker_control_bus: BusSystemLocal<WorkerControlIn<TCfg>>,
    worker_control_recv: VecDeque<(BusLegReceiver<WorkerControlIn<TCfg>>, usize)>,
    worker_event_bus: BusSystemBuilder<WorkerControlOut>,
    worker_event_recv: BusLegReceiver<WorkerControlOut>,
    _tmp: std::marker::PhantomData<(ExtIn, ExtOut, MSG, T, B)>,
    worker_threads: Vec<WorkerContainer>,
}

impl<
        ExtIn,
        ExtOut,
        MSG: 'static + Send + Sync + Clone,
        T: Task<ExtIn, ExtOut, MSG, TCfg>,
        TCfg: 'static + Send + Sync,
        B: Backend<usize>,
    > Controller<ExtIn, ExtOut, MSG, T, TCfg, B>
{
    pub fn new(workers: usize) -> Self {
        let mut worker_control_bus = BusSystemBuilder::default();
        let mut worker_control_recv = VecDeque::new();
        for _ in 0..workers {
            worker_control_recv.push_back(worker_control_bus.new_leg());
        }
        let mut worker_event_bus = BusSystemBuilder::default();
        let (worker_event_recv, current_leg_index) = worker_event_bus.new_leg();

        Self {
            workers,
            worker_control_bus: worker_control_bus.build_local(current_leg_index),
            worker_control_recv,
            worker_event_bus: worker_event_bus,
            worker_event_recv,
            _tmp: Default::default(),
            worker_threads: Vec::new(),
        }
    }

    pub fn start(&mut self) {
        log::info!("Starting {} workers", self.workers);
        let mut internal_bus = BusSystemBuilder::default();
        let mut internal_recv = VecDeque::new();
        for _ in 0..self.workers {
            internal_recv.push_back(internal_bus.new_leg());
        }

        for i in 1..=self.workers {
            let (worker_control_recv, worker_event_index) = self
                .worker_control_recv
                .pop_front()
                .expect("Should have worker control recv");
            let worker_event_bus = self.worker_event_bus.build_local(worker_event_index);
            let (internal_recv, internal_leg_index) = internal_recv
                .pop_front()
                .expect("Should have internal recv");
            let internal_bus = internal_bus.build_local(internal_leg_index);
            let join = std::thread::spawn(move || {
                log::info!("Worker {} started", i);
                let mut worker = worker::Worker::<ExtIn, ExtOut, MSG, T, TCfg, B>::new(
                    worker_event_bus,
                    worker_control_recv,
                    internal_bus,
                    internal_recv,
                );
                loop {
                    worker.process();
                }
            });
            self.worker_threads.push(WorkerContainer {
                _join: join,
                stats: Default::default(),
            });
        }
    }

    pub fn process(&mut self) {
        while let Some(event) = self.worker_event_recv.recv() {
            match event {
                BusLegEvent::Direct(source_leg, WorkerControlOut::Stats(stats)) => {
                    log::trace!("Worker stats: {:?}", stats);
                    self.worker_threads[source_leg].stats = stats;
                }
                _ => {
                    unreachable!("WorkerControlOut only has Stats variant");
                }
            }
        }
    }

    pub fn spawn(&mut self, cfg: TCfg) {
        // get worker index with lowest load
        let best_worker = self
            .worker_threads
            .iter()
            .enumerate()
            .min_by_key(|(_, w)| w.stats.load())
            .map(|(i, _)| i)
            .expect("Should have at least one worker");
        self.worker_threads[best_worker].stats.tasks += 1;
        if let Err(e) = self
            .worker_control_bus
            .send(best_worker, WorkerControlIn::Spawn(cfg))
        {
            log::error!("Failed to spawn task: {:?}", e);
        }
    }
}
