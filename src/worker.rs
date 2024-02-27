use std::{
    fmt::Debug,
    hash::Hash,
    time::{Duration, Instant},
};

use crate::{
    backend::Backend,
    bus::{BusSendSingleFeature, BusWorker},
    owner::Owner,
    BackendOwner, BusEvent, BusEventSource, BusLocalHub, BusPubSubFeature, NetIncoming,
    NetOutgoing,
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

pub enum WorkerInnerOutput<'a, ExtOut, ChannelId, Event> {
    Net(Owner, NetOutgoing<'a>),
    Bus(Owner, BusEvent<ChannelId, Event>),
    DestroyOwner(Owner),
    /// First bool is message need to safe to send or not, second is the message
    Ext(bool, ExtOut),
}

pub struct WorkerCtx<'a> {
    pub(crate) backend: &'a mut dyn BackendOwner,
}

pub trait WorkerInner<ExtIn, ExtOut, ChannelId, Event, ICfg, SCfg> {
    fn build(worker: u16, cfg: ICfg) -> Self;
    fn worker_index(&self) -> u16;
    fn tasks(&self) -> usize;
    fn spawn(&mut self, now: Instant, ctx: &mut WorkerCtx<'_>, cfg: SCfg);
    fn on_ext(&mut self, now: Instant, ctx: &mut WorkerCtx<'_>, ext: ExtIn);
    fn on_bus(
        &mut self,
        now: Instant,
        ctx: &mut WorkerCtx<'_>,
        owner: Owner,
        channel_id: ChannelId,
        event: Event,
    );
    fn on_net(&mut self, now: Instant, owner: Owner, net: NetIncoming);
    fn inner_process(&mut self, now: Instant, ctx: &mut WorkerCtx<'_>);
    fn pop_output(&mut self) -> Option<WorkerInnerOutput<'_, ExtOut, ChannelId, Event>>;
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
            _tmp: Default::default(),
        }
    }

    pub fn process(&mut self) {
        let now = Instant::now();
        let mut ctx = WorkerCtx {
            backend: &mut self.backend,
        };
        while let Some((_source, msg)) = self.worker_in.recv() {
            match msg {
                WorkerControlIn::Ext(ext) => {
                    self.inner.on_ext(now, &mut ctx, ext);
                }
                WorkerControlIn::Spawn(cfg) => {
                    self.inner.spawn(now, &mut ctx, cfg);
                }
                WorkerControlIn::StatsRequest => {
                    let stats = WorkerStats {
                        tasks: self.inner.tasks(),
                        ultilization: 0, //TODO measure this thread ultilization
                    };
                    if let Err(e) = self
                        .worker_out
                        .send(0, true, WorkerControlOut::Stats(stats))
                    {
                        log::error!("Failed to send stats: {:?}", e);
                    }
                }
            }
        }

        while let Some((source, event)) = self.inner_bus.recv() {
            match source {
                BusEventSource::Channel(_, channel) => {
                    if let Some(subscribers) = self.bus_local_hub.get_subscribers(channel) {
                        for subscriber in subscribers {
                            self.inner
                                .on_bus(now, &mut ctx, *subscriber, channel, event.clone());
                        }
                    }
                }
                _ => panic!(
                    "Invalid channel source for task {:?}, only support channel",
                    source
                ),
            }
        }

        self.inner.inner_process(now, &mut ctx);
        while let Some(output) = self.inner.pop_output() {
            match output {
                WorkerInnerOutput::Net(owner, event) => {
                    self.backend.on_action(owner, event);
                }
                WorkerInnerOutput::Bus(owner, event) => match event {
                    BusEvent::ChannelSubscribe(channel) => {
                        if self.bus_local_hub.subscribe(owner, channel) {
                            log::info!(
                                "Worker {} subscribe to channel {:?}",
                                self.inner.worker_index(),
                                channel
                            );
                            self.inner_bus.subscribe(channel);
                        }
                    }
                    BusEvent::ChannelUnsubscribe(channel) => {
                        if self.bus_local_hub.unsubscribe(owner, channel) {
                            log::info!(
                                "Worker {} unsubscribe from channel {:?}",
                                self.inner.worker_index(),
                                channel
                            );
                            self.inner_bus.unsubscribe(channel);
                        }
                    }
                    BusEvent::ChannelPublish(channel, safe, msg) => {
                        self.inner_bus.publish(channel, safe, msg);
                    }
                },
                WorkerInnerOutput::Ext(safe, ext) => {
                    // TODO dont hardcode 0
                    if let Err(e) = self.worker_out.send(0, safe, WorkerControlOut::Ext(ext)) {
                        log::error!("Failed to send external: {:?}", e);
                    }
                }
                WorkerInnerOutput::DestroyOwner(owner) => {
                    self.backend.remove_owner(owner);
                    self.bus_local_hub.remove_owner(owner);
                }
            }
        }

        self.backend.finish_outgoing_cycle();

        //one cycle is process in 1ms then we minus 1ms with eslaped time
        let mut remain_time = Duration::from_millis(1)
            .checked_sub(now.elapsed())
            .unwrap_or_else(|| Duration::from_micros(1));

        while let Some((event, owner)) = self.backend.pop_incoming(remain_time) {
            self.inner.on_net(now, owner, event);
            remain_time = Duration::from_millis(1)
                .checked_sub(now.elapsed())
                .unwrap_or_else(|| Duration::from_micros(1));
        }

        self.backend.finish_incoming_cycle();
        log::debug!("Worker process done in {}", now.elapsed().as_nanos());
    }
}
