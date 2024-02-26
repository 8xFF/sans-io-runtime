use std::time::{Duration, Instant};

use crate::{
    backend::Backend,
    bus::{BusLegReceiver, BusSendSingleFeature, BusSystemLocal},
    owner::Owner,
    BackendOwner, NetIncoming, NetOutgoing,
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

pub enum WorkerInnerOutput<'a, Ext> {
    Net(NetOutgoing<'a>, Owner),
    External(Ext),
}

pub struct WorkerCtx<'a> {
    pub(crate) backend: &'a mut dyn BackendOwner,
}

pub trait WorkerInner<ExtIn, ExtOut, ICfg, SCfg> {
    fn build(worker: u16, cfg: ICfg) -> Self;
    fn tasks(&self) -> usize;
    fn spawn(&mut self, now: Instant, ctx: &mut WorkerCtx<'_>, cfg: SCfg);
    fn on_ext(&mut self, now: Instant, ctx: &mut WorkerCtx<'_>, ext: ExtIn);
    fn on_net(&mut self, now: Instant, owner: Owner, net: NetIncoming);
    fn inner_process(&mut self, now: Instant, ctx: &mut WorkerCtx<'_>);
    fn pop_output(&mut self) -> Option<WorkerInnerOutput<'_, ExtOut>>;
}

pub(crate) struct Worker<
    ExtIn: Clone,
    ExtOut: Clone,
    Inner: WorkerInner<ExtIn, ExtOut, ICfg, SCfg>,
    ICfg,
    SCfg: Clone,
    B: Backend,
> {
    inner: Inner,
    backend: B,
    worker_out_bus: BusSystemLocal<u16, WorkerControlOut<ExtOut>>,
    control_recv: BusLegReceiver<u16, WorkerControlIn<ExtIn, SCfg>>,
    _tmp: std::marker::PhantomData<(Owner, ICfg)>,
}

impl<
        ExtIn: Clone,
        ExtOut: Clone,
        Inner: WorkerInner<ExtIn, ExtOut, ICfg, SCfg>,
        B: Backend,
        ICfg,
        SCfg: Clone,
    > Worker<ExtIn, ExtOut, Inner, ICfg, SCfg, B>
{
    pub fn new(
        inner: Inner,
        worker_out_bus: BusSystemLocal<u16, WorkerControlOut<ExtOut>>,
        control_recv: BusLegReceiver<u16, WorkerControlIn<ExtIn, SCfg>>,
    ) -> Self {
        Self {
            inner,
            backend: Default::default(),
            worker_out_bus,
            control_recv,
            _tmp: Default::default(),
        }
    }

    pub fn process(&mut self) {
        let now = Instant::now();
        let mut ctx = WorkerCtx {
            backend: &mut self.backend,
        };
        while let Some((_source, msg)) = self.control_recv.recv() {
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
                    if let Err(e) = self.worker_out_bus.send(0, WorkerControlOut::Stats(stats)) {
                        log::error!("Failed to send stats: {:?}", e);
                    }
                }
            }
        }

        self.inner.inner_process(now, &mut ctx);
        while let Some(output) = self.inner.pop_output() {
            match output {
                WorkerInnerOutput::Net(net, owner) => {
                    self.backend.on_action(net, owner);
                }
                WorkerInnerOutput::External(ext) => {
                    if let Err(e) = self.worker_out_bus.send(0, WorkerControlOut::Ext(ext)) {
                        log::error!("Failed to send external: {:?}", e);
                    }
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
