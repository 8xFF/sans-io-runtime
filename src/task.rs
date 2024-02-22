use std::{net::SocketAddr, time::Instant};

use crate::backend::SansIoBackendEvent;

pub enum SansIoInput<'a, Ext, MSG> {
    Backend(SansIoBackendEvent<'a>),
    Bus(MSG),
    External(Ext),
}

pub enum SansIoOutput<'a, Ext, MSG> {
    Backend(SansIoBackendEvent<'a>),
    Bus(MSG),
    External(Ext),
}

pub trait SansIoTask<ExtIn, ExtOut, MSG> {
    fn min_tick_interval(&self) -> std::time::Duration;
    fn on_tick(&mut self, now: Instant);
    fn on_input<'a>(&mut self, now: Instant, input: SansIoInput<'a, ExtIn, MSG>);
    fn pop_output<'a>(&mut self, now: Instant) -> Option<SansIoOutput<'a, ExtOut, MSG>>;
}
