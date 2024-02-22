use crate::{
    backend::SansIoBackend,
    bus::{BusLegReceiver, BusSystem},
    task::SansIoTask,
};

pub struct SansIoWorker<
    ExtIn,
    ExtOut,
    MSG: Send + Sync + Clone,
    T: SansIoTask<ExtIn, ExtOut, MSG>,
    B: SansIoBackend,
> {
    tasks: Vec<T>,
    backend: B,
    bug_system: BusSystem<MSG>,
    bus_recv: BusLegReceiver<MSG>,
    _tmp: std::marker::PhantomData<(ExtIn, ExtOut)>,
}

impl<
        ExtIn,
        ExtOut,
        MSG: Send + Sync + Clone,
        T: SansIoTask<ExtIn, ExtOut, MSG>,
        B: SansIoBackend,
    > SansIoWorker<ExtIn, ExtOut, MSG, T, B>
{
    pub fn new(bus_system: BusSystem<MSG>, bus_recv: BusLegReceiver<MSG>) -> Self {
        Self {
            tasks: Vec::new(),
            backend: Default::default(),
            bug_system: bus_system,
            bus_recv,
            _tmp: Default::default(),
        }
    }

    pub fn process(&mut self) {}
}
