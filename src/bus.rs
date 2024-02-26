use parking_lot::RwLock;
use std::{collections::HashMap, hash::Hash, sync::Arc};

mod leg;
mod local_hub;

pub use leg::*;

pub enum BusEvent<ChannelId, MSG> {
    ChannelSubscribe(ChannelId),
    ChannelUnsubscribe(ChannelId),
    ChannelPublish(ChannelId, MSG),
    Broadcast(MSG),
    Direct(usize, MSG),
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
    legs: Vec<BusLegSender<ChannelId, MSG>>,
}

impl<ChannelId, MSG> Default for BusSystemBuilder<ChannelId, MSG> {
    fn default() -> Self {
        Self { legs: Vec::new() }
    }
}

impl<ChannelId, MSG> BusSystemBuilder<ChannelId, MSG> {
    pub fn new_leg(&mut self) -> (BusLegReceiver<ChannelId, MSG>, usize) {
        let leg_index = self.legs.len();
        let (sender, recv) = create_bus_leg();
        self.legs.push(sender);
        (recv, leg_index)
    }

    pub fn build_local(&self, local_leg_index: usize) -> BusSystemLocal<ChannelId, MSG> {
        BusSystemLocal {
            local_leg_index,
            legs: self.legs.clone(),
            channels: Default::default(),
        }
    }
}

impl<ChannelId, MSG> BusSendSingleFeature<MSG> for BusSystemBuilder<ChannelId, MSG> {
    fn send(&self, dest_leg: usize, msg: MSG) -> Result<usize, BusLegSenderErr> {
        self.legs[dest_leg].send(BusEventSource::External, msg)
    }
}

impl<ChannelId, MSG: Clone> BusSendMultiFeature<MSG> for BusSystemBuilder<ChannelId, MSG> {
    fn broadcast(&self, msg: MSG) {
        for leg in &self.legs {
            let _ = leg.send(BusEventSource::External, msg.clone());
        }
    }
}

#[derive(Debug, Default)]
pub struct BusSystemLocal<ChannelId, MSG> {
    local_leg_index: usize,
    legs: Vec<BusLegSender<ChannelId, MSG>>,
    channels: Arc<RwLock<HashMap<ChannelId, Vec<usize>>>>,
}

impl<ChannelId, MSG> BusSendSingleFeature<MSG> for BusSystemLocal<ChannelId, MSG> {
    fn send(&self, dest_leg: usize, msg: MSG) -> Result<usize, BusLegSenderErr> {
        self.legs[dest_leg].send(BusEventSource::Direct(self.local_leg_index), msg)
    }
}

impl<ChannelId, MSG: Clone> BusSendMultiFeature<MSG> for BusSystemLocal<ChannelId, MSG> {
    fn broadcast(&self, msg: MSG) {
        for leg in &self.legs {
            let _ = leg.send(BusEventSource::Broadcast(self.local_leg_index), msg.clone());
        }
    }
}

impl<ChannelId: Copy + Hash + PartialEq + Eq, MSG: Clone> BusPubSubFeature<ChannelId, MSG>
    for BusSystemLocal<ChannelId, MSG>
{
    fn subscribe(&self, channel: ChannelId) {
        let mut channels = self.channels.write();
        let entry = channels.entry(channel).or_insert_with(Vec::new);
        if entry.contains(&self.local_leg_index) {
            return;
        }
        entry.push(self.local_leg_index);
    }

    fn unsubscribe(&self, channel: ChannelId) {
        let mut channels = self.channels.write();
        if let Some(entry) = channels.get_mut(&channel) {
            if let Some(index) = entry.iter().position(|x| *x == self.local_leg_index) {
                entry.swap_remove(index);
            }
        }
    }

    fn publish(&self, channel: ChannelId, msg: MSG) {
        let channels = self.channels.read();
        if let Some(entry) = channels.get(&channel) {
            for &leg_index in entry {
                let _ = self.legs[leg_index].send(
                    BusEventSource::Channel(self.local_leg_index, channel),
                    msg.clone(),
                );
            }
        }
    }
}
