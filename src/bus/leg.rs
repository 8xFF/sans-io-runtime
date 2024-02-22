use std::{collections::VecDeque, sync::Arc};

use parking_lot::Mutex;

use super::BusChannelId;

const BUS_CHANNEL_PRE_ALOC: usize = 64;
const BUS_CHANNEL_SIZE_LIMIT: usize = 1024;

#[derive(Debug, Clone)]
pub enum BusLegEvent<MSG: Clone> {
    Channel(BusChannelId, MSG),
    Broadcast(MSG),
    Direct(MSG),
}

pub enum BusLegSenderErr {
    ChannelFull,
}

#[derive(Debug, Clone)]
pub struct BusLegSender<MSG: Clone> {
    queue: Arc<Mutex<VecDeque<BusLegEvent<MSG>>>>,
}

impl<MSG: Clone> BusLegSender<MSG> {
    pub fn send(&self, msg: BusLegEvent<MSG>) -> Result<usize, BusLegSenderErr> {
        let mut queue = self.queue.lock();
        if queue.len() >= BUS_CHANNEL_SIZE_LIMIT {
            return Err(BusLegSenderErr::ChannelFull);
        }
        queue.push_back(msg);
        Ok(queue.len())
    }
}

pub struct BusLegReceiver<MSG: Clone> {
    queue: Arc<Mutex<VecDeque<BusLegEvent<MSG>>>>,
}

impl<MSG: Clone> BusLegReceiver<MSG> {
    pub fn recv(&self) -> Option<BusLegEvent<MSG>> {
        self.queue.lock().pop_front()
    }
}

pub fn create_bus_leg<MSG: Clone>() -> (BusLegSender<MSG>, BusLegReceiver<MSG>) {
    let queue = Arc::new(Mutex::new(VecDeque::with_capacity(BUS_CHANNEL_PRE_ALOC)));
    let sender = BusLegSender {
        queue: queue.clone(),
    };
    let receiver = BusLegReceiver { queue };
    (sender, receiver)
}
