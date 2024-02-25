use std::collections::VecDeque;

use crate::{
    backend::Backend,
    bus::{
        BusLegEvent, BusLegReceiver, BusMultiDest, BusSingleDest, BusSystemBuilder, BusSystemLocal,
    },
    worker::{self, WorkerControlIn, WorkerControlOut, WorkerInner, WorkerStats},
};

struct WorkerContainer {
    _join: std::thread::JoinHandle<()>,
    stats: WorkerStats,
}

pub struct Controller<ExtIn: Clone, ExtOut: Clone, SCfg: Clone> {
    worker_control_bus: BusSystemLocal<u16, WorkerControlIn<ExtIn, SCfg>>,
    worker_control_recv: VecDeque<(BusLegReceiver<u16, WorkerControlIn<ExtIn, SCfg>>, usize)>,
    worker_event_bus: BusSystemBuilder<u16, WorkerControlOut<ExtOut>>,
    worker_event_recv: BusLegReceiver<u16, WorkerControlOut<ExtOut>>,
    worker_threads: Vec<WorkerContainer>,
}

impl<
        ExtIn: 'static + Send + Sync + Clone,
        ExtOut: 'static + Send + Sync + Clone,
        SCfg: 'static + Send + Sync + Clone,
    > Controller<ExtIn, ExtOut, SCfg>
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
            worker_control_bus: worker_control_bus.build_local(current_leg_index),
            worker_control_recv,
            worker_event_bus: worker_event_bus,
            worker_event_recv,
            worker_threads: Vec::new(),
        }
    }

    pub fn add_worker<
        Owner: Copy + PartialEq + Eq,
        ICfg: 'static + Send + Sync,
        Inner: WorkerInner<ExtIn, ExtOut, ICfg, SCfg, Owner>,
        B: Backend<Owner>,
    >(
        &mut self,
        cfg: ICfg,
    ) {
        let (worker_control_recv, worker_event_index) = self
            .worker_control_recv
            .pop_front()
            .expect("Should have worker control recv");
        let worker_event_bus = self.worker_event_bus.build_local(worker_event_index);
        let join = std::thread::spawn(move || {
            let mut worker = worker::Worker::<ExtIn, ExtOut, Inner, ICfg, SCfg, B, Owner>::new(
                Inner::build(cfg),
                worker_event_bus,
                worker_control_recv,
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

    pub fn process(&mut self) {
        self.worker_control_bus
            .broadcast(WorkerControlIn::StatsRequest);
        while let Some(event) = self.worker_event_recv.recv() {
            match event {
                BusLegEvent::Direct(source_leg, WorkerControlOut::Stats(stats)) => {
                    log::info!("Worker stats: {:?}", stats);
                    self.worker_threads[source_leg].stats = stats;
                }
                _ => {
                    unreachable!("WorkerControlOut only has Stats variant");
                }
            }
        }
    }

    pub fn spawn(&mut self, cfg: SCfg) {
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
