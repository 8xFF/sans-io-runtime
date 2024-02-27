use std::{fmt::Debug, hash::Hash};

use crate::{
    backend::Backend,
    bus::{BusEventSource, BusSendMultiFeature, BusSendSingleFeature, BusSystemBuilder},
    worker::{self, WorkerControlIn, WorkerControlOut, WorkerInner, WorkerStats},
    BusWorker,
};

struct WorkerContainer {
    _join: std::thread::JoinHandle<()>,
    stats: WorkerStats,
}

pub struct Controller<ExtIn: Clone, ExtOut: Clone, SCfg: Clone, ChannelId, Event> {
    worker_inner_bus: BusSystemBuilder<ChannelId, Event>,
    worker_control_bus: BusSystemBuilder<u16, WorkerControlIn<ExtIn, SCfg>>,
    worker_event_bus: BusSystemBuilder<u16, WorkerControlOut<ExtOut>>,
    worker_event: BusWorker<u16, WorkerControlOut<ExtOut>>,
    worker_threads: Vec<WorkerContainer>,
}

impl<
        ExtIn: 'static + Send + Sync + Clone,
        ExtOut: 'static + Send + Sync + Clone,
        SCfg: 'static + Send + Sync + Clone,
        ChannelId: 'static + Debug + Clone + Copy + Hash + PartialEq + Eq + Send + Sync,
        Event: 'static + Send + Sync + Clone,
    > Controller<ExtIn, ExtOut, SCfg, ChannelId, Event>
{
    pub fn new() -> Self {
        let worker_control_bus = BusSystemBuilder::default();
        let mut worker_event_bus = BusSystemBuilder::default();
        let worker_event = worker_event_bus.new_worker();

        Self {
            worker_inner_bus: BusSystemBuilder::default(),
            worker_control_bus,
            worker_event_bus,
            worker_event,
            worker_threads: Vec::new(),
        }
    }

    pub fn add_worker<
        ICfg: 'static + Send + Sync,
        Inner: WorkerInner<ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg>,
        B: Backend,
    >(
        &mut self,
        cfg: ICfg,
    ) {
        let worker_in = self.worker_control_bus.new_worker();
        let worker_out = self.worker_event_bus.new_worker();
        let worker_inner = self.worker_inner_bus.new_worker();

        let join = std::thread::spawn(move || {
            let mut worker =
                worker::Worker::<ExtIn, ExtOut, ChannelId, Event, Inner, ICfg, SCfg, B>::new(
                    Inner::build(worker_in.leg_index() as u16, cfg),
                    worker_inner,
                    worker_out,
                    worker_in,
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
        while let Some((source, event)) = self.worker_event.recv() {
            match (source, event) {
                (BusEventSource::Direct(source_leg), WorkerControlOut::Stats(stats)) => {
                    log::debug!("Worker stats: {:?}", stats);
                    // source_leg is 1-based because of 0 is for controller
                    // TODO avoid -1, should not hack this way
                    self.worker_threads[source_leg - 1].stats = stats;
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
