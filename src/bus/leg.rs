use std::{collections::VecDeque, sync::Arc};

use parking_lot::Mutex;

const BUS_CHANNEL_PRE_ALOC: usize = 64;
const BUS_CHANNEL_SIZE_LIMIT: usize = 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum BusEventSource<ChannelId> {
    Channel(usize, ChannelId),
    Broadcast(usize),
    Direct(usize),
    External,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BusLegSenderErr {
    ChannelFull,
}

#[derive(Debug)]
pub struct BusLegSender<ChannelId, MSG> {
    queue: Arc<Mutex<VecDeque<(BusEventSource<ChannelId>, MSG)>>>,
}

impl<ChannelId, MSG> Clone for BusLegSender<ChannelId, MSG> {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
        }
    }
}

impl<ChannelId, MSG> BusLegSender<ChannelId, MSG> {
    pub fn send(
        &self,
        source: BusEventSource<ChannelId>,
        msg: MSG,
    ) -> Result<usize, BusLegSenderErr> {
        let mut queue = self.queue.lock();
        if queue.len() >= BUS_CHANNEL_SIZE_LIMIT {
            return Err(BusLegSenderErr::ChannelFull);
        }
        queue.push_back((source, msg));
        Ok(queue.len())
    }
}

pub struct BusLegReceiver<ChannelId, MSG> {
    queue: Arc<Mutex<VecDeque<(BusEventSource<ChannelId>, MSG)>>>,
}

impl<ChannelId, MSG> BusLegReceiver<ChannelId, MSG> {
    pub fn recv(&self) -> Option<(BusEventSource<ChannelId>, MSG)> {
        self.queue.lock().pop_front()
    }
}

pub fn create_bus_leg<ChannelId, MSG>(
) -> (BusLegSender<ChannelId, MSG>, BusLegReceiver<ChannelId, MSG>) {
    let queue = Arc::new(Mutex::new(VecDeque::with_capacity(BUS_CHANNEL_PRE_ALOC)));
    let sender = BusLegSender {
        queue: queue.clone(),
    };
    let receiver = BusLegReceiver { queue };
    (sender, receiver)
}
