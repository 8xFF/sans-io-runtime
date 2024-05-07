use std::{
    fmt::Debug,
    hash::Hash,
    time::{Duration, Instant},
};

use crate::{
    backend::{Backend, BackendIncoming, BackendIncomingInternal, BackendOutgoing},
    bus::{
        BusEventSource, BusLocalHub, BusPubSubFeature, BusSendMultiFeature, BusSendSingleFeature,
        BusWorker,
    },
};

#[derive(Debug)]
pub enum BusChannelControl<ChannelId, MSG> {
    Subscribe(ChannelId),
    Unsubscribe(ChannelId),
    /// The first parameter is the channel id, the second parameter is whether the message is safe, and the third parameter is the message.
    Publish(ChannelId, bool, MSG),
}

impl<ChannelId, MSG> BusChannelControl<ChannelId, MSG> {
    pub fn convert_into<NChannelId: From<ChannelId>, NMSG: From<MSG>>(
        self,
    ) -> BusChannelControl<NChannelId, NMSG> {
        match self {
            Self::Subscribe(channel) => BusChannelControl::Subscribe(channel.into()),
            Self::Unsubscribe(channel) => BusChannelControl::Unsubscribe(channel.into()),
            Self::Publish(channel, safe, msg) => {
                BusChannelControl::Publish(channel.into(), safe, msg.into())
            }
        }
    }
}

#[derive(Debug)]
pub enum BusControl<Owner, ChannelId, MSG> {
    Channel(Owner, BusChannelControl<ChannelId, MSG>),
    Broadcast(bool, MSG),
}

impl<Owner, ChannelId, MSG> BusControl<Owner, ChannelId, MSG> {
    pub fn high_priority(&self) -> bool {
        matches!(
            self,
            Self::Channel(_, BusChannelControl::Subscribe(..))
                | Self::Channel(_, BusChannelControl::Unsubscribe(..))
        )
    }

    pub fn convert_into<NOwner: From<Owner>, NChannelId: From<ChannelId>, NMSG: From<MSG>>(
        self,
    ) -> BusControl<Owner, NChannelId, NMSG> {
        match self {
            Self::Channel(owner, event) => BusControl::Channel(owner, event.convert_into()),
            Self::Broadcast(safe, msg) => BusControl::Broadcast(safe, msg.into()),
        }
    }
}

#[derive(Debug)]
pub enum BusEvent<Owner, ChannelId, MSG> {
    Broadcast(u16, MSG),
    Channel(Owner, ChannelId, MSG),
}

#[derive(Debug, Clone)]
pub enum WorkerControlIn<Ext: Clone, SCfg> {
    Ext(Ext),
    Spawn(SCfg),
    StatsRequest,
    Shutdown,
}

#[derive(Debug)]
pub enum WorkerControlOut<Ext, SCfg> {
    Stats(WorkerStats),
    Ext(Ext),
    Spawn(SCfg),
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

    pub fn tasks(&self) -> usize {
        self.tasks
    }
}

#[derive(Debug)]
pub enum WorkerInnerInput<Owner, ExtIn, ChannelId, Event> {
    Net(Owner, BackendIncoming),
    Bus(BusEvent<Owner, ChannelId, Event>),
    Ext(ExtIn),
}

pub enum WorkerInnerOutput<Owner, ExtOut, ChannelId, Event, SCfg> {
    Net(Owner, BackendOutgoing),
    Bus(BusControl<Owner, ChannelId, Event>),
    /// First bool is message need to safe to send or not, second is the message
    Ext(bool, ExtOut),
    Spawn(SCfg),
    Destroy(Owner),
    Continue,
}

pub trait WorkerInner<Owner, ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg> {
    fn build(worker: u16, cfg: ICfg) -> Self;
    fn worker_index(&self) -> u16;
    fn tasks(&self) -> usize;
    fn spawn(&mut self, now: Instant, cfg: SCfg);
    fn on_tick(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<Owner, ExtOut, ChannelId, Event, SCfg>>;
    fn on_event(
        &mut self,
        now: Instant,
        event: WorkerInnerInput<Owner, ExtIn, ChannelId, Event>,
    ) -> Option<WorkerInnerOutput<Owner, ExtOut, ChannelId, Event, SCfg>>;
    fn pop_output(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<Owner, ExtOut, ChannelId, Event, SCfg>>;
    fn shutdown(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<Owner, ExtOut, ChannelId, Event, SCfg>>;
}

pub(crate) struct Worker<
    Owner,
    ExtIn: Clone,
    ExtOut: Clone,
    ChannelId: Hash + PartialEq + Eq,
    Event,
    Inner: WorkerInner<Owner, ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg>,
    ICfg,
    SCfg,
    B: Backend<Owner>,
    const INNER_BUS_STACK: usize,
> {
    tick: Duration,
    last_tick: Instant,
    inner: Inner,
    bus_local_hub: BusLocalHub<Owner, ChannelId>,
    inner_bus: BusWorker<ChannelId, Event, INNER_BUS_STACK>,
    backend: B,
    worker_out: BusWorker<u16, WorkerControlOut<ExtOut, SCfg>, INNER_BUS_STACK>,
    worker_in: BusWorker<u16, WorkerControlIn<ExtIn, SCfg>, INNER_BUS_STACK>,
    _tmp: std::marker::PhantomData<(Owner, ICfg)>,
}

impl<
        Owner: Debug + Clone + Copy + PartialEq,
        ExtIn: Clone,
        ExtOut: Clone,
        ChannelId: Hash + PartialEq + Eq + Debug + Copy,
        Event: Clone,
        Inner: WorkerInner<Owner, ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg>,
        B: Backend<Owner>,
        ICfg,
        SCfg,
        const INNER_BUS_STACK: usize,
    > Worker<Owner, ExtIn, ExtOut, ChannelId, Event, Inner, ICfg, SCfg, B, INNER_BUS_STACK>
{
    pub fn new(
        tick: Duration,
        inner: Inner,
        inner_bus: BusWorker<ChannelId, Event, INNER_BUS_STACK>,
        worker_out: BusWorker<u16, WorkerControlOut<ExtOut, SCfg>, INNER_BUS_STACK>,
        worker_in: BusWorker<u16, WorkerControlIn<ExtIn, SCfg>, INNER_BUS_STACK>,
    ) -> Self {
        let backend = B::default();
        inner_bus.set_awaker(backend.create_awaker());
        Self {
            inner,
            tick,
            last_tick: Instant::now() - 2 * tick, //for ensure first tick as fast as possible
            bus_local_hub: Default::default(),
            inner_bus,
            backend,
            worker_out,
            worker_in,
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
                WorkerControlIn::Shutdown => {
                    Self::on_shutdown(
                        now,
                        &mut self.inner,
                        &mut self.inner_bus,
                        &mut self.worker_out,
                        &mut self.backend,
                        &mut self.bus_local_hub,
                    );
                }
            }
        }

        if now.duration_since(self.last_tick) >= self.tick {
            self.last_tick = now;
            Self::on_input_tick(
                now,
                &mut self.inner,
                &mut self.inner_bus,
                &mut self.worker_out,
                &mut self.backend,
                &mut self.bus_local_hub,
            );
        }

        //one cycle is process in 1ms then we minus 1ms with eslaped time
        let remain_time = Duration::from_millis(1)
            .checked_sub(now.elapsed())
            .unwrap_or_else(|| Duration::from_micros(1));

        self.backend.poll_incoming(remain_time);

        while let Some(event) = self.backend.pop_incoming() {
            match event {
                BackendIncomingInternal::Awake => self.on_awake(now),
                BackendIncomingInternal::Event(owner, event) => {
                    Self::on_input_event(
                        now,
                        WorkerInnerInput::Net(owner, event),
                        &mut self.inner,
                        &mut self.inner_bus,
                        &mut self.worker_out,
                        &mut self.backend,
                        &mut self.bus_local_hub,
                    );
                }
            }
        }

        self.backend.finish_incoming_cycle();
        self.backend.finish_outgoing_cycle();
        if now.elapsed().as_millis() > 15 {
            log::warn!("Worker process too long: {}", now.elapsed().as_millis());
        }
    }

    fn on_input_tick(
        now: Instant,
        inner: &mut Inner,
        inner_bus: &mut BusWorker<ChannelId, Event, INNER_BUS_STACK>,
        worker_out: &mut BusWorker<u16, WorkerControlOut<ExtOut, SCfg>, INNER_BUS_STACK>,
        backend: &mut B,
        bus_local_hub: &mut BusLocalHub<Owner, ChannelId>,
    ) {
        let worker = inner.worker_index();
        while let Some(out) = inner.on_tick(now) {
            Self::process_inner_output(out, worker, inner_bus, worker_out, backend, bus_local_hub);
            while let Some(out) = inner.pop_output(now) {
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

    fn on_shutdown(
        now: Instant,
        inner: &mut Inner,
        inner_bus: &mut BusWorker<ChannelId, Event, INNER_BUS_STACK>,
        worker_out: &mut BusWorker<u16, WorkerControlOut<ExtOut, SCfg>, INNER_BUS_STACK>,
        backend: &mut B,
        bus_local_hub: &mut BusLocalHub<Owner, ChannelId>,
    ) {
        let worker = inner.worker_index();
        while let Some(out) = inner.shutdown(now) {
            log::info!("Process shutdown");
            Self::process_inner_output(out, worker, inner_bus, worker_out, backend, bus_local_hub);
            while let Some(out) = inner.pop_output(now) {
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

    fn on_awake(&mut self, now: Instant) {
        while let Some((source, event)) = self.inner_bus.recv() {
            match source {
                BusEventSource::Channel(_, channel) => {
                    if let Some(subscribers) = self.bus_local_hub.get_subscribers(channel) {
                        for subscriber in subscribers {
                            Self::on_input_event(
                                now,
                                WorkerInnerInput::Bus(BusEvent::Channel(
                                    subscriber,
                                    channel,
                                    event.clone(),
                                )),
                                &mut self.inner,
                                &mut self.inner_bus,
                                &mut self.worker_out,
                                &mut self.backend,
                                &mut self.bus_local_hub,
                            );
                        }
                    }
                }
                BusEventSource::Broadcast(from) => {
                    Self::on_input_event(
                        now,
                        WorkerInnerInput::Bus(BusEvent::Broadcast(from as u16, event.clone())),
                        &mut self.inner,
                        &mut self.inner_bus,
                        &mut self.worker_out,
                        &mut self.backend,
                        &mut self.bus_local_hub,
                    );
                }
                _ => panic!(
                    "Invalid channel source for task {:?}, only support channel",
                    source
                ),
            }
        }
    }

    fn on_input_event(
        now: Instant,
        input: WorkerInnerInput<Owner, ExtIn, ChannelId, Event>,
        inner: &mut Inner,
        inner_bus: &mut BusWorker<ChannelId, Event, INNER_BUS_STACK>,
        worker_out: &mut BusWorker<u16, WorkerControlOut<ExtOut, SCfg>, INNER_BUS_STACK>,
        backend: &mut B,
        bus_local_hub: &mut BusLocalHub<Owner, ChannelId>,
    ) {
        let worker = inner.worker_index();
        if let Some(out) = inner.on_event(now, input) {
            Self::process_inner_output(out, worker, inner_bus, worker_out, backend, bus_local_hub);
            while let Some(out) = inner.pop_output(now) {
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

    fn process_inner_output(
        out: WorkerInnerOutput<Owner, ExtOut, ChannelId, Event, SCfg>,
        worker: u16,
        inner_bus: &mut BusWorker<ChannelId, Event, INNER_BUS_STACK>,
        worker_out: &mut BusWorker<u16, WorkerControlOut<ExtOut, SCfg>, INNER_BUS_STACK>,
        backend: &mut B,
        bus_local_hub: &mut BusLocalHub<Owner, ChannelId>,
    ) {
        match out {
            WorkerInnerOutput::Spawn(cfg) => {
                worker_out
                    .send(0, true, WorkerControlOut::Spawn(cfg))
                    .expect("Should send success with safe flag");
            }
            WorkerInnerOutput::Net(owner, action) => {
                backend.on_action(owner, action);
            }
            WorkerInnerOutput::Bus(event) => match event {
                BusControl::Channel(owner, BusChannelControl::Subscribe(channel)) => {
                    if bus_local_hub.subscribe(owner, channel) {
                        log::info!("Worker {worker} subscribe to channel {:?}", channel);
                        inner_bus.subscribe(channel);
                    }
                }
                BusControl::Channel(owner, BusChannelControl::Unsubscribe(channel)) => {
                    if bus_local_hub.unsubscribe(owner, channel) {
                        log::info!("Worker {worker} unsubscribe from channel {:?}", channel);
                        inner_bus.unsubscribe(channel);
                    }
                }
                BusControl::Channel(_owner, BusChannelControl::Publish(channel, safe, msg)) => {
                    inner_bus.publish(channel, safe, msg);
                }
                BusControl::Broadcast(safe, msg) => {
                    inner_bus.broadcast(safe, msg);
                }
            },
            WorkerInnerOutput::Destroy(owner) => {
                log::info!("Worker {worker} destroy owner {:?}", owner);
                backend.remove_owner(owner);
                bus_local_hub.remove_owner(owner);
            }
            WorkerInnerOutput::Ext(safe, ext) => {
                // TODO don't hardcode 0
                log::debug!("Worker {worker} send external");
                if let Err(e) = worker_out.send(0, safe, WorkerControlOut::Ext(ext)) {
                    log::error!("Failed to send external: {:?}", e);
                }
            }
            WorkerInnerOutput::Continue => {}
        }
    }
}
