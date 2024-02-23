use std::{collections::VecDeque, sync::Arc};

use parking_lot::Mutex;

use super::BusChannelId;

const BUS_CHANNEL_PRE_ALOC: usize = 64;
const BUS_CHANNEL_SIZE_LIMIT: usize = 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum BusLegEvent<MSG> {
    Channel(usize, BusChannelId, MSG),
    Broadcast(usize, MSG),
    Direct(usize, MSG),
}

#[derive(Debug, PartialEq, Eq)]
pub enum BusLegSenderErr {
    ChannelFull,
}

#[derive(Debug)]
pub struct BusLegSender<MSG> {
    queue: Arc<Mutex<VecDeque<BusLegEvent<MSG>>>>,
}

impl<MSG> Clone for BusLegSender<MSG> {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
        }
    }
}

impl<MSG> BusLegSender<MSG> {
    pub fn send(&self, msg: BusLegEvent<MSG>) -> Result<usize, BusLegSenderErr> {
        let mut queue = self.queue.lock();
        if queue.len() >= BUS_CHANNEL_SIZE_LIMIT {
            return Err(BusLegSenderErr::ChannelFull);
        }
        queue.push_back(msg);
        Ok(queue.len())
    }
}

pub struct BusLegReceiver<MSG> {
    queue: Arc<Mutex<VecDeque<BusLegEvent<MSG>>>>,
}

impl<MSG> BusLegReceiver<MSG> {
    pub fn recv(&self) -> Option<BusLegEvent<MSG>> {
        self.queue.lock().pop_front()
    }
}

pub fn create_bus_leg<MSG>() -> (BusLegSender<MSG>, BusLegReceiver<MSG>) {
    let queue = Arc::new(Mutex::new(VecDeque::with_capacity(BUS_CHANNEL_PRE_ALOC)));
    let sender = BusLegSender {
        queue: queue.clone(),
    };
    let receiver = BusLegReceiver { queue };
    (sender, receiver)
}
