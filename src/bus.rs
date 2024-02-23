use parking_lot::RwLock;
use std::{collections::HashMap, ops::Deref, sync::Arc};

mod leg;
mod local_hub;

pub use leg::*;
pub use local_hub::*;

#[derive(Hash, PartialEq, Eq, Debug, Clone, Copy)]
pub struct BusChannelId(pub u64);

impl Deref for BusChannelId {
    type Target = u64;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub enum BusEvent<MSG> {
    ChannelSubscribe(BusChannelId),
    ChannelUnsubscribe(BusChannelId),
    ChannelPublish(BusChannelId, MSG),
    Broadcast(MSG),
    Direct(usize, MSG),
}

#[derive(Debug)]
pub struct BusSystemBuilder<MSG> {
    legs: Vec<BusLegSender<MSG>>,
}

impl<MSG> Default for BusSystemBuilder<MSG> {
    fn default() -> Self {
        Self { legs: Vec::new() }
    }
}

impl<MSG> BusSystemBuilder<MSG> {
    pub fn new_leg(&mut self) -> (BusLegReceiver<MSG>, usize) {
        let leg_index = self.legs.len();
        let (sender, recv) = create_bus_leg();
        self.legs.push(sender);
        (recv, leg_index)
    }

    pub fn build_local(&self, local_leg_index: usize) -> BusSystemLocal<MSG> {
        BusSystemLocal {
            local_leg_index,
            legs: self.legs.clone(),
            channels: Default::default(),
        }
    }
}

pub trait BusSingleDest<MSG> {
    fn send(&self, dest_leg: usize, msg: MSG) -> Result<usize, BusLegSenderErr>;
}

pub trait BusMultiDest<MSG: Clone> {
    fn subscribe(&self, channel: BusChannelId);
    fn unsubscribe(&self, channel: BusChannelId);
    fn publish(&self, channel: BusChannelId, msg: MSG);
    fn broadcast(&self, msg: MSG);
}

#[derive(Debug, Default)]
pub struct BusSystemLocal<MSG> {
    local_leg_index: usize,
    legs: Vec<BusLegSender<MSG>>,
    channels: Arc<RwLock<HashMap<BusChannelId, Vec<usize>>>>,
}

impl<MSG> BusSingleDest<MSG> for BusSystemLocal<MSG> {
    fn send(&self, dest_leg: usize, msg: MSG) -> Result<usize, BusLegSenderErr> {
        self.legs[dest_leg].send(BusLegEvent::Direct(self.local_leg_index, msg))
    }
}

impl<MSG: Clone> BusMultiDest<MSG> for BusSystemLocal<MSG> {
    fn subscribe(&self, channel: BusChannelId) {
        let mut channels = self.channels.write();
        let entry = channels.entry(channel).or_insert_with(Vec::new);
        if entry.contains(&self.local_leg_index) {
            return;
        }
        entry.push(self.local_leg_index);
    }

    fn unsubscribe(&self, channel: BusChannelId) {
        let mut channels = self.channels.write();
        if let Some(entry) = channels.get_mut(&channel) {
            if let Some(index) = entry.iter().position(|x| *x == self.local_leg_index) {
                entry.swap_remove(index);
            }
        }
    }

    fn publish(&self, channel: BusChannelId, msg: MSG) {
        let channels = self.channels.read();
        if let Some(entry) = channels.get(&channel) {
            for &leg_index in entry {
                let _ = self.legs[leg_index].send(BusLegEvent::Channel(
                    self.local_leg_index,
                    channel,
                    msg.clone(),
                ));
            }
        }
    }

    fn broadcast(&self, msg: MSG) {
        for leg in &self.legs {
            let _ = leg.send(BusLegEvent::Broadcast(self.local_leg_index, msg.clone()));
        }
    }
}
