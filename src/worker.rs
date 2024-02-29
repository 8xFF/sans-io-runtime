use std::{
    fmt::Debug,
    hash::Hash,
    time::{Duration, Instant},
};

use crate::{
    backend::Backend,
    bus::{
        BusEvent, BusEventSource, BusLocalHub, BusPubSubFeature, BusSendSingleFeature, BusWorker,
    },
    owner::Owner,
    NetIncoming, TaskInput, TaskOutput,
};

#[derive(Debug, Clone)]
pub enum WorkerControlIn<Ext: Clone, SCfg: Clone> {
    Ext(Ext),
    Spawn(SCfg),
    StatsRequest,
}

#[derive(Debug)]
pub enum WorkerControlOut<Ext> {
    Stats(WorkerStats),
    Ext(Ext),
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

pub enum WorkerInnerInput<'a, ExtIn, ChannelId, Event> {
    Task(Owner, TaskInput<'a, ChannelId, Event>),
    Ext(ExtIn),
}

pub enum WorkerInnerOutput<'a, ExtOut, ChannelId, Event> {
    Task(Owner, TaskOutput<'a, ChannelId, Event>),
    /// First bool is message need to safe to send or not, second is the message
    Ext(bool, ExtOut),
}

pub trait WorkerInner<ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg> {
    fn build(worker: u16, cfg: ICfg) -> Self;
    fn worker_index(&self) -> u16;
    fn tasks(&self) -> usize;
    fn spawn(&mut self, now: Instant, cfg: SCfg);
    fn on_input_tick<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event>>;
    fn on_input_event<'a>(
        &mut self,
        now: Instant,
        event: WorkerInnerInput<'a, ExtIn, ChannelId, Event>,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event>>;
    fn pop_last_input<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event>>;
    fn pop_output<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, Event>>;
}

pub(crate) struct Worker<
    ExtIn: Clone,
    ExtOut: Clone,
    ChannelId: Hash + PartialEq + Eq,
    Event,
    Inner: WorkerInner<ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg>,
    ICfg,
    SCfg: Clone,
    B: Backend,
    const INNER_BUS_STACK: usize,
> {
    inner: Inner,
    bus_local_hub: BusLocalHub<ChannelId>,
    inner_bus: BusWorker<ChannelId, Event, INNER_BUS_STACK>,
    backend: B,
    worker_out: BusWorker<u16, WorkerControlOut<ExtOut>, 16>,
    worker_in: BusWorker<u16, WorkerControlIn<ExtIn, SCfg>, 16>,
    network_buffer: [u8; 8096],
    _tmp: std::marker::PhantomData<(Owner, ICfg)>,
}

impl<
        ExtIn: Clone,
        ExtOut: Clone,
        ChannelId: Hash + PartialEq + Eq + Debug + Copy,
        Event: Clone,
        Inner: WorkerInner<ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg>,
        B: Backend,
        ICfg,
        SCfg: Clone,
        const INNER_BUS_STACK: usize,
    > Worker<ExtIn, ExtOut, ChannelId, Event, Inner, ICfg, SCfg, B, INNER_BUS_STACK>
{
    pub fn new(
        inner: Inner,
        inner_bus: BusWorker<ChannelId, Event, INNER_BUS_STACK>,
        worker_out: BusWorker<u16, WorkerControlOut<ExtOut>, 16>,
        worker_in: BusWorker<u16, WorkerControlIn<ExtIn, SCfg>, 16>,
    ) -> Self {
        Self {
            inner,
            bus_local_hub: Default::default(),
            inner_bus,
            backend: Default::default(),
            worker_out,
            worker_in,
            network_buffer: [0; 8096],
            _tmp: Default::default(),
        }
    }

    pub fn process(&mut self) {
        let now = Instant::now();
        while let Some((_source, msg)) = self.worker_in.recv() {
            match msg {
                WorkerControlIn::Ext(ext) => {
                    Self::on_input_event(
                        now,
                        WorkerInnerInput::Ext(ext),
                        &mut self.inner,
                        &mut self.inner_bus,
                        &mut self.worker_out,
                        &mut self.backend,
                        &mut self.bus_local_hub,
                    );
                }
                WorkerControlIn::Spawn(cfg) => {
                    self.inner.spawn(now, cfg);
                }
                WorkerControlIn::StatsRequest => {
                    let stats = WorkerStats {
                        tasks: self.inner.tasks(),
                        ultilization: 0, //TODO measure this thread ultilization
                    };
                    self.worker_out
                        .send(0, true, WorkerControlOut::Stats(stats))
                        .expect("Should send success with safe flag");
                }
            }
        }

        while let Some((source, event)) = self.inner_bus.recv() {
            match source {
                BusEventSource::Channel(_, channel) => {
                    if let Some(subscribers) = self.bus_local_hub.get_subscribers(channel) {
                        for subscriber in subscribers {
                            Self::on_input_event(
                                now,
                                WorkerInnerInput::Task(
                                    subscriber,
                                    TaskInput::Bus(channel, event.clone()),
                                ),
                                &mut self.inner,
                                &mut self.inner_bus,
                                &mut self.worker_out,
                                &mut self.backend,
                                &mut self.bus_local_hub,
                            );
                        }
                    }
                }
                _ => panic!(
                    "Invalid channel source for task {:?}, only support channel",
                    source
                ),
            }
        }

        Self::on_input_tick(
            now,
            &mut self.inner,
            &mut self.inner_bus,
            &mut self.worker_out,
            &mut self.backend,
            &mut self.bus_local_hub,
        );

        //one cycle is process in 1ms then we minus 1ms with eslaped time
        let mut remain_time = Duration::from_millis(1)
            .checked_sub(now.elapsed())
            .unwrap_or_else(|| Duration::from_micros(1));

        while let Some((event, owner)) = self
            .backend
            .pop_incoming(remain_time, &mut self.network_buffer)
        {
            let event = NetIncoming::from_backend(event, &self.network_buffer);
            Self::on_input_event(
                now,
                WorkerInnerInput::Task(owner, TaskInput::Net(event)),
                &mut self.inner,
                &mut self.inner_bus,
                &mut self.worker_out,
                &mut self.backend,
                &mut self.bus_local_hub,
            );
            remain_time = Duration::from_millis(1)
                .checked_sub(now.elapsed())
                .unwrap_or_else(|| Duration::from_micros(1));
        }

        self.backend.finish_incoming_cycle();

        let worker = self.inner.worker_index();
        while let Some(output) = self.inner.pop_output(now) {
            Self::process_inner_output(
                output,
                worker,
                &mut self.inner_bus,
                &mut self.worker_out,
                &mut self.backend,
                &mut self.bus_local_hub,
            );
        }

        self.backend.finish_outgoing_cycle();
        log::debug!("Worker process done in {}", now.elapsed().as_nanos());
    }

    fn on_input_tick<'a>(
        now: Instant,
        inner: &mut Inner,
        inner_bus: &mut BusWorker<ChannelId, Event, INNER_BUS_STACK>,
        worker_out: &mut BusWorker<u16, WorkerControlOut<ExtOut>, 16>,
        backend: &mut B,
        bus_local_hub: &mut BusLocalHub<ChannelId>,
    ) {
        let worker = inner.worker_index();
        while let Some(out) = inner.on_input_tick(now) {
            Self::process_inner_output(out, worker, inner_bus, worker_out, backend, bus_local_hub);
            while let Some(out) = inner.pop_last_input(now) {
                Self::process_inner_output(
                    out,
                    worker,
                    inner_bus,
                    worker_out,
                    backend,
                    bus_local_hub,
                );
            }
        }
    }

    fn on_input_event<'a>(
        now: Instant,
        input: WorkerInnerInput<'a, ExtIn, ChannelId, Event>,
        inner: &mut Inner,
        inner_bus: &mut BusWorker<ChannelId, Event, INNER_BUS_STACK>,
        worker_out: &mut BusWorker<u16, WorkerControlOut<ExtOut>, 16>,
        backend: &mut B,
        bus_local_hub: &mut BusLocalHub<ChannelId>,
    ) {
        let worker = inner.worker_index();
        if let Some(out) = inner.on_input_event(now, input) {
            Self::process_inner_output(out, worker, inner_bus, worker_out, backend, bus_local_hub);
            while let Some(out) = inner.pop_last_input(now) {
                Self::process_inner_output(
                    out,
                    worker,
                    inner_bus,
                    worker_out,
                    backend,
                    bus_local_hub,
                );
            }
        }
    }

    fn process_inner_output<'a>(
        out: WorkerInnerOutput<'a, ExtOut, ChannelId, Event>,
        worker: u16,
        inner_bus: &mut BusWorker<ChannelId, Event, INNER_BUS_STACK>,
        worker_out: &mut BusWorker<u16, WorkerControlOut<ExtOut>, 16>,
        backend: &mut B,
        bus_local_hub: &mut BusLocalHub<ChannelId>,
    ) {
        match out {
            WorkerInnerOutput::Task(owner, TaskOutput::Net(event)) => {
                backend.on_action(owner, event);
            }
            WorkerInnerOutput::Task(owner, TaskOutput::Bus(event)) => match event {
                BusEvent::ChannelSubscribe(channel) => {
                    if bus_local_hub.subscribe(owner, channel) {
                        log::info!("Worker {worker} subscribe to channel {:?}", channel);
                        inner_bus.subscribe(channel);
                    }
                }
                BusEvent::ChannelUnsubscribe(channel) => {
                    if bus_local_hub.unsubscribe(owner, channel) {
                        log::info!("Worker {worker} unsubscribe from channel {:?}", channel);
                        inner_bus.unsubscribe(channel);
                    }
                }
                BusEvent::ChannelPublish(channel, safe, msg) => {
                    inner_bus.publish(channel, safe, msg);
                }
            },
            WorkerInnerOutput::Task(owner, TaskOutput::Destroy) => {
                log::info!("Worker {worker} destroy owner {:?}", owner);
                backend.remove_owner(owner);
                bus_local_hub.remove_owner(owner);
            }
            WorkerInnerOutput::Ext(safe, ext) => {
                // TODO don't hardcode 0
                log::info!("Worker {worker} send external");
                if let Err(e) = worker_out.send(0, safe, WorkerControlOut::Ext(ext)) {
                    log::error!("Failed to send external: {:?}", e);
                }
            }
        }
    }
}
