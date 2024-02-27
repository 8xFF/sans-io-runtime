use parking_lot::RwLock;
use std::{collections::HashMap, hash::Hash, sync::Arc};

mod leg;
mod local_hub;

pub use leg::*;
pub use local_hub::*;

pub enum BusEvent<ChannelId, MSG> {
    ChannelSubscribe(ChannelId),
    ChannelUnsubscribe(ChannelId),
    ChannelPublish(ChannelId, MSG),
}

pub trait BusSendSingleFeature<MSG> {
    fn send(&self, dest_leg: usize, msg: MSG) -> Result<usize, BusLegSenderErr>;
}

pub trait BusSendMultiFeature<MSG: Clone> {
    fn broadcast(&self, msg: MSG);
}

pub trait BusPubSubFeature<ChannelId, MSG: Clone> {
    fn subscribe(&self, channel: ChannelId);
    fn unsubscribe(&self, channel: ChannelId);
    fn publish(&self, channel: ChannelId, msg: MSG);
}

#[derive(Debug)]
pub struct BusSystemBuilder<ChannelId, MSG> {
    legs: Arc<RwLock<Vec<BusLegSender<ChannelId, MSG>>>>,
}

impl<ChannelId, MSG> Default for BusSystemBuilder<ChannelId, MSG> {
    fn default() -> Self {
        Self {
            legs: Default::default(),
        }
    }
}

impl<ChannelId, MSG> BusSystemBuilder<ChannelId, MSG> {
    pub fn new_worker(&mut self) -> BusWorker<ChannelId, MSG> {
        let mut legs = self.legs.write();
        let leg_index = legs.len();
        let (sender, recv) = create_bus_leg();
        legs.push(sender);

        BusWorker {
            leg_index,
            receiver: recv,
            legs: self.legs.clone(),
            channels: Default::default(),
        }
    }
}

impl<ChannelId, MSG> BusSendSingleFeature<MSG> for BusSystemBuilder<ChannelId, MSG> {
    fn send(&self, dest_leg: usize, msg: MSG) -> Result<usize, BusLegSenderErr> {
        let legs = self.legs.read();
        legs[dest_leg].send(BusEventSource::External, msg)
    }
}

impl<ChannelId, MSG: Clone> BusSendMultiFeature<MSG> for BusSystemBuilder<ChannelId, MSG> {
    fn broadcast(&self, msg: MSG) {
        let legs = self.legs.read();
        for leg in &*legs {
            let _ = leg.send(BusEventSource::External, msg.clone());
        }
    }
}

pub struct BusWorker<ChannelId, MSG> {
    leg_index: usize,
    receiver: BusLegReceiver<ChannelId, MSG>,
    legs: Arc<RwLock<Vec<BusLegSender<ChannelId, MSG>>>>,
    channels: Arc<RwLock<HashMap<ChannelId, Vec<usize>>>>,
}

impl<ChannelId, MSG> BusWorker<ChannelId, MSG> {
    pub fn leg_index(&self) -> usize {
        self.leg_index
    }

    pub fn recv(&self) -> Option<(BusEventSource<ChannelId>, MSG)> {
        self.receiver.recv()
    }
}

impl<ChannelId, MSG> BusSendSingleFeature<MSG> for BusWorker<ChannelId, MSG> {
    fn send(&self, dest_leg: usize, msg: MSG) -> Result<usize, BusLegSenderErr> {
        let legs = self.legs.read();
        legs[dest_leg].send(BusEventSource::Direct(self.leg_index), msg)
    }
}

impl<ChannelId, MSG: Clone> BusSendMultiFeature<MSG> for BusWorker<ChannelId, MSG> {
    fn broadcast(&self, msg: MSG) {
        let legs = self.legs.read();
        for leg in &*legs {
            let _ = leg.send(BusEventSource::Broadcast(self.leg_index), msg.clone());
        }
    }
}

impl<ChannelId: Copy + Hash + PartialEq + Eq, MSG: Clone> BusPubSubFeature<ChannelId, MSG>
    for BusWorker<ChannelId, MSG>
{
    fn subscribe(&self, channel: ChannelId) {
        let mut channels = self.channels.write();
        let entry = channels.entry(channel).or_insert_with(Vec::new);
        if entry.contains(&self.leg_index) {
            return;
        }
        entry.push(self.leg_index);
    }

    fn unsubscribe(&self, channel: ChannelId) {
        let mut channels = self.channels.write();
        if let Some(entry) = channels.get_mut(&channel) {
            if let Some(index) = entry.iter().position(|x| *x == self.leg_index) {
                entry.swap_remove(index);
            }
        }
    }

    fn publish(&self, channel: ChannelId, msg: MSG) {
        let legs = self.legs.read();
        let channels = self.channels.read();
        if let Some(entry) = channels.get(&channel) {
            for &leg_index in entry {
                let _ = legs[leg_index].send(
                    BusEventSource::Channel(self.leg_index, channel),
                    msg.clone(),
                );
            }
        }
    }
}
