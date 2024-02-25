use std::time::{Duration, Instant};

use crate::{
    backend::Backend,
    bus::{BusLegEvent, BusLegReceiver, BusSingleDest, BusSystemLocal},
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

pub enum WorkerInnerOutput<'a, Ext, Owner> {
    Net(NetOutgoing<'a>, Owner),
    External(Ext),
}

pub struct WorkerCtx<'a, Owner> {
    backend: &'a mut dyn BackendOwner<Owner>,
}

pub trait WorkerInner<ExtIn, ExtOut, ICfg, SCfg, Owner> {
    fn build(cfg: ICfg) -> Self;
    fn tasks(&self) -> usize;
    fn spawn(&mut self, ctx: WorkerCtx<'_, Owner>, cfg: SCfg);
    fn on_ext(&mut self, ext: ExtIn);
    fn on_net(&mut self, owner: Owner, net: NetIncoming);
    fn inner_process(&mut self);
    fn pop_output(&mut self) -> Option<WorkerInnerOutput<'_, ExtOut, Owner>>;
}

pub(crate) struct Worker<
    ExtIn: Clone,
    ExtOut: Clone,
    Inner: WorkerInner<ExtIn, ExtOut, ICfg, SCfg, Owner>,
    ICfg,
    SCfg: Clone,
    B: Backend<Owner>,
    Owner: Copy + PartialEq + Eq,
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
        Inner: WorkerInner<ExtIn, ExtOut, ICfg, SCfg, Owner>,
        B: Backend<Owner>,
        ICfg,
        SCfg: Clone,
        Owner: Copy + PartialEq + Eq,
    > Worker<ExtIn, ExtOut, Inner, ICfg, SCfg, B, Owner>
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
        while let Some(control) = self.control_recv.recv() {
            let msg = match control {
                BusLegEvent::Direct(_source_leg, msg) => msg,
                BusLegEvent::Broadcast(_, msg) => msg,
                _ => {
                    unreachable!("WorkerControlIn only has Spawn variant");
                }
            };
            match msg {
                WorkerControlIn::Ext(ext) => {
                    self.inner.on_ext(ext);
                }
                WorkerControlIn::Spawn(cfg) => {
                    self.inner.spawn(
                        WorkerCtx {
                            backend: &mut self.backend,
                        },
                        cfg,
                    );
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

        self.inner.inner_process();
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
            self.inner.on_net(owner, event);
            remain_time = Duration::from_millis(1)
                .checked_sub(now.elapsed())
                .unwrap_or_else(|| Duration::from_micros(1));
        }

        self.backend.finish_incoming_cycle();
        log::debug!("Worker process done in {}", now.elapsed().as_nanos());
    }
}
