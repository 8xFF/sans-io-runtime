use crate::{
    backend::Backend,
    bus::{
        BusEventSource, BusLegReceiver, BusSendMultiFeature, BusSendSingleFeature, BusSystemBuilder,
    },
    worker::{self, WorkerControlIn, WorkerControlOut, WorkerInner, WorkerStats},
};

struct WorkerContainer {
    _join: std::thread::JoinHandle<()>,
    stats: WorkerStats,
}

pub struct Controller<ExtIn: Clone, ExtOut: Clone, SCfg: Clone> {
    worker_control_bus: BusSystemBuilder<u16, WorkerControlIn<ExtIn, SCfg>>,
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
    pub fn new() -> Self {
        let worker_control_bus = BusSystemBuilder::default();
        let mut worker_event_bus = BusSystemBuilder::default();
        let (worker_event_recv, _current_leg_index) = worker_event_bus.new_leg();

        Self {
            worker_control_bus,
            worker_event_bus,
            worker_event_recv,
            worker_threads: Vec::new(),
        }
    }

    pub fn add_worker<
        ICfg: 'static + Send + Sync,
        Inner: WorkerInner<ExtIn, ExtOut, ICfg, SCfg>,
        B: Backend,
    >(
        &mut self,
        cfg: ICfg,
    ) {
        let (worker_control_recv, worker_event_index) = self.worker_control_bus.new_leg();
        let worker_event_bus = self.worker_event_bus.build_local(worker_event_index);
        let join = std::thread::spawn(move || {
            let mut worker = worker::Worker::<ExtIn, ExtOut, Inner, ICfg, SCfg, B>::new(
                Inner::build(worker_event_index as u16, cfg),
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
        while let Some((source, event)) = self.worker_event_recv.recv() {
            match (source, event) {
                (BusEventSource::Direct(source_leg), WorkerControlOut::Stats(stats)) => {
                    log::debug!("Worker stats: {:?}", stats);
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
