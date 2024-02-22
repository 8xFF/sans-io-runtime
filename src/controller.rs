use std::collections::VecDeque;

use crate::{
    backend::SansIoBackend,
    bus::{BusLegReceiver, BusSystem, BusSystemBuilder},
    task::SansIoTask,
    worker,
};

pub struct SansIoController<
    ExtIn,
    ExtOut,
    MSG: 'static + Clone + Send + Sync,
    T: SansIoTask<ExtIn, ExtOut, MSG>,
    B: SansIoBackend,
> {
    workers: usize,
    bus_system: BusSystem<MSG>,
    bus_recv: BusLegReceiver<MSG>,
    worker_bus_recv: VecDeque<BusLegReceiver<MSG>>,
    _tmp: std::marker::PhantomData<(ExtIn, ExtOut, T, B)>,
    worker_threads: Vec<std::thread::JoinHandle<()>>,
}

impl<
        ExtIn,
        ExtOut,
        MSG: 'static + Send + Sync + Clone,
        T: SansIoTask<ExtIn, ExtOut, MSG>,
        B: SansIoBackend,
    > SansIoController<ExtIn, ExtOut, MSG, T, B>
{
    pub fn new(workers: usize) -> Self {
        let mut bus = BusSystemBuilder::default();
        let bus_recv = bus.new_leg();
        let mut worker_bus_recv = VecDeque::with_capacity(workers);
        for i in 0..workers {
            worker_bus_recv.push_back(bus.new_leg());
        }

        Self {
            workers,
            bus_system: bus.build(),
            bus_recv,
            worker_bus_recv,
            _tmp: Default::default(),
            worker_threads: Vec::new(),
        }
    }

    pub fn start(&mut self) {
        log::info!("Starting {} workers", self.workers);
        for i in 1..=self.workers {
            let bus_system = self.bus_system.clone();
            let bus_recv = self
                .worker_bus_recv
                .pop_front()
                .expect("Should has bus recv");
            let thread = std::thread::spawn(move || {
                log::info!("Worker {} started", i);
                let mut worker =
                    worker::SansIoWorker::<ExtIn, ExtOut, MSG, T, B>::new(bus_system, bus_recv);
                loop {
                    worker.process();
                }
            });
            self.worker_threads.push(thread);
        }
    }
}
