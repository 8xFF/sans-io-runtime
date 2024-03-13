use std::{fmt::Debug, hash::Hash};

use crate::{
    backend::Backend,
    bus::{BusEventSource, BusSendSingleFeature, BusSystemBuilder, BusWorker},
    collections::DynamicDeque,
    worker::{self, WorkerControlIn, WorkerControlOut, WorkerInner, WorkerStats},
    Owner,
};

const DEFAULT_STACK_SIZE: usize = 12 * 1024 * 1024;

enum State {
    Running,
    Shutdowning,
    Shutdowned,
}

struct WorkerContainer {
    _join: std::thread::JoinHandle<()>,
    stats: WorkerStats,
}

pub struct Controller<
    ExtIn: Clone,
    ExtOut: Clone,
    SCfg,
    ChannelId,
    Event,
    const INNER_BUS_STACK: usize,
> {
    worker_inner_bus: BusSystemBuilder<ChannelId, Event, INNER_BUS_STACK>,
    worker_control_bus: BusSystemBuilder<u16, WorkerControlIn<ExtIn, SCfg>, 16>,
    worker_event_bus: BusSystemBuilder<u16, WorkerControlOut<ExtOut, SCfg>, 16>,
    worker_event: BusWorker<u16, WorkerControlOut<ExtOut, SCfg>, 16>,
    worker_threads: Vec<WorkerContainer>,
    output: DynamicDeque<ExtOut, 16>,
    state: State,
}

impl<ExtIn: Clone, ExtOut: Clone, SCfg, ChannelId, Event, const INNER_BUS_STACK: usize> Default
    for Controller<ExtIn, ExtOut, SCfg, ChannelId, Event, INNER_BUS_STACK>
{
    fn default() -> Self {
        let worker_control_bus = BusSystemBuilder::default();
        let mut worker_event_bus = BusSystemBuilder::default();
        let worker_event = worker_event_bus.new_worker();

        Self {
            worker_inner_bus: BusSystemBuilder::default(),
            worker_control_bus,
            worker_event_bus,
            worker_event,
            worker_threads: Vec::new(),
            output: DynamicDeque::default(),
            state: State::Running,
        }
    }
}

impl<
        ExtIn: 'static + Send + Sync + Clone,
        ExtOut: 'static + Send + Sync + Clone,
        SCfg: 'static + Send + Sync,
        ChannelId: 'static + Debug + Clone + Copy + Hash + PartialEq + Eq + Send + Sync,
        Event: 'static + Send + Sync + Clone,
        const INNER_BUS_STACK: usize,
    > Controller<ExtIn, ExtOut, SCfg, ChannelId, Event, INNER_BUS_STACK>
{
    pub fn add_worker<
        ICfg: 'static + Send + Sync,
        Inner: WorkerInner<ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg>,
        B: Backend,
    >(
        &mut self,
        cfg: ICfg,
        stack_size: Option<usize>,
    ) {
        let worker_in = self.worker_control_bus.new_worker();
        let worker_out = self.worker_event_bus.new_worker();
        let worker_inner = self.worker_inner_bus.new_worker();

        let stack_size = stack_size.unwrap_or(DEFAULT_STACK_SIZE);
        log::info!("create worker with stack size: {}", stack_size);
        let join = std::thread::Builder::new()
            .stack_size(stack_size)
            .spawn(move || {
                let mut worker = worker::Worker::<_, _, _, _, _, _, _, B, INNER_BUS_STACK>::new(
                    Inner::build(worker_in.leg_index() as u16, cfg),
                    worker_inner,
                    worker_out,
                    worker_in,
                );
                loop {
                    worker.process();
                }
            })
            .expect("Should spawn worker thread");
        self.worker_threads.push(WorkerContainer {
            _join: join,
            stats: Default::default(),
        });
    }

    pub fn process(&mut self) -> Option<()> {
        if matches!(self.state, State::Shutdowned) {
            return None;
        }
        if matches!(self.state, State::Shutdowning) {
            if self.worker_threads.iter().all(|w| w.stats.tasks() == 0) {
                self.state = State::Shutdowned;
                return None;
            }
        }

        for i in 0..self.worker_threads.len() {
            if let Err(e) = self
                .worker_control_bus
                .send(i, false, WorkerControlIn::StatsRequest)
            {
                log::error!("Failed to send stats request to worker {i}: {:?}", e);
            }
        }
        while let Some((source, event)) = self.worker_event.recv() {
            match (source, event) {
                (BusEventSource::Direct(source_leg), event) => match event {
                    WorkerControlOut::Stats(stats) => {
                        log::trace!("Worker {source_leg} stats: {:?}", stats);
                        // source_leg is 1-based because of 0 is for controller
                        // TODO avoid -1, should not hack this way
                        self.worker_threads[source_leg - 1].stats = stats;
                    }
                    WorkerControlOut::Ext(event) => {
                        self.output.push_back_safe(event);
                    }
                    WorkerControlOut::Spawn(cfg) => {
                        self.spawn(cfg);
                    }
                },
                _ => {
                    unreachable!("WorkerControlOut only has Stats variant");
                }
            }
        }

        Some(())
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
            .send(best_worker, true, WorkerControlIn::Spawn(cfg))
        {
            log::error!("Failed to spawn task: {:?}", e);
        }
    }

    pub fn shutdown(&mut self) {
        match self.state {
            State::Shutdowned | State::Shutdowning => {
                return;
            }
            State::Running => {}
        }
        self.state = State::Shutdowning;
        for i in 0..self.worker_threads.len() {
            if let Err(e) = self
                .worker_control_bus
                .send(i, true, WorkerControlIn::Shutdown)
            {
                log::error!("Failed to send shutdown to task: {:?}", e);
            }
        }
    }

    pub fn send_to(&mut self, owner: Owner, ext: ExtIn) {
        if let Err(e) = self.worker_control_bus.send(
            owner.worker_id() as usize,
            true,
            WorkerControlIn::Ext(ext),
        ) {
            log::error!("Failed to send to task: {:?}", e);
        }
    }

    pub fn pop_event(&mut self) -> Option<ExtOut> {
        self.output.pop_front()
    }
}
