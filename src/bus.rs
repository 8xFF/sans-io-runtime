use parking_lot::RwLock;
use std::{collections::HashMap, fmt::Debug, hash::Hash, sync::Arc};

mod leg;
mod local_hub;

pub use leg::*;
pub use local_hub::*;

pub enum BusEvent<ChannelId, MSG> {
    ChannelSubscribe(ChannelId),
    ChannelUnsubscribe(ChannelId),
    /// The first parameter is the channel id, the second parameter is whether the message is safe, and the third parameter is the message.
    ChannelPublish(ChannelId, bool, MSG),
}

impl<ChannelId, MSG> BusEvent<ChannelId, MSG> {
    pub fn high_priority(&self) -> bool {
        match self {
            Self::ChannelSubscribe(_) => true,
            Self::ChannelUnsubscribe(_) => true,
            Self::ChannelPublish(_, _, _) => false,
        }
    }
}

pub trait BusSendSingleFeature<MSG> {
    fn send_safe(&self, dest_leg: usize, msg: MSG) -> usize;
    fn send(&self, dest_leg: usize, safe: bool, msg: MSG) -> Result<usize, BusLegSenderErr>;
}

pub trait BusSendMultiFeature<MSG: Clone> {
    fn broadcast(&self, safe: bool, msg: MSG);
}

pub trait BusPubSubFeature<ChannelId, MSG: Clone> {
    fn subscribe(&self, channel: ChannelId);
    fn unsubscribe(&self, channel: ChannelId);
    fn publish(&self, channel: ChannelId, safe: bool, msg: MSG);
}

pub struct BusSystemBuilder<ChannelId, MSG, const STACK_SIZE: usize> {
    legs: Arc<RwLock<Vec<BusLegSender<ChannelId, MSG, STACK_SIZE>>>>,
    channels: Arc<RwLock<HashMap<ChannelId, Vec<usize>>>>,
}

impl<ChannelId, MSG, const STACK_SIZE: usize> Default
    for BusSystemBuilder<ChannelId, MSG, STACK_SIZE>
{
    fn default() -> Self {
        Self {
            legs: Default::default(),
            channels: Default::default(),
        }
    }
}

impl<ChannelId, MSG, const STACK_SIZE: usize> BusSystemBuilder<ChannelId, MSG, STACK_SIZE> {
    pub fn new_worker(&mut self) -> BusWorker<ChannelId, MSG, STACK_SIZE> {
        let mut legs = self.legs.write();
        let leg_index = legs.len();
        let (sender, recv) = create_bus_leg();
        legs.push(sender);

        BusWorker {
            leg_index,
            receiver: recv,
            legs: self.legs.clone(),
            channels: self.channels.clone(),
        }
    }
}

impl<ChannelId, MSG, const STACK_SIZE: usize> BusSendSingleFeature<MSG>
    for BusSystemBuilder<ChannelId, MSG, STACK_SIZE>
{
    fn send_safe(&self, dest_leg: usize, msg: MSG) -> usize {
        let legs = self.legs.read();
        legs[dest_leg].send_safe(BusEventSource::External, msg)
    }

    fn send(&self, dest_leg: usize, safe: bool, msg: MSG) -> Result<usize, BusLegSenderErr> {
        let legs = self.legs.read();
        legs[dest_leg].send(BusEventSource::External, safe, msg)
    }
}

impl<ChannelId, MSG: Clone, const STACK_SIZE: usize> BusSendMultiFeature<MSG>
    for BusSystemBuilder<ChannelId, MSG, STACK_SIZE>
{
    fn broadcast(&self, safe: bool, msg: MSG) {
        let legs = self.legs.read();
        for leg in &*legs {
            let _ = leg.send(BusEventSource::External, safe, msg.clone());
        }
    }
}

pub struct BusWorker<ChannelId, MSG, const STACK_SIZE: usize> {
    leg_index: usize,
    receiver: BusLegReceiver<ChannelId, MSG, STACK_SIZE>,
    legs: Arc<RwLock<Vec<BusLegSender<ChannelId, MSG, STACK_SIZE>>>>,
    channels: Arc<RwLock<HashMap<ChannelId, Vec<usize>>>>,
}

impl<ChannelId, MSG, const STACK_SIZE: usize> BusWorker<ChannelId, MSG, STACK_SIZE> {
    pub fn leg_index(&self) -> usize {
        self.leg_index
    }

    pub fn recv(&self) -> Option<(BusEventSource<ChannelId>, MSG)> {
        self.receiver.recv()
    }
}

impl<ChannelId, MSG, const STACK_SIZE: usize> BusSendSingleFeature<MSG>
    for BusWorker<ChannelId, MSG, STACK_SIZE>
{
    fn send_safe(&self, dest_leg: usize, msg: MSG) -> usize {
        let legs = self.legs.read();
        legs[dest_leg].send_safe(BusEventSource::Direct(self.leg_index), msg)
    }

    fn send(&self, dest_leg: usize, safe: bool, msg: MSG) -> Result<usize, BusLegSenderErr> {
        let legs = self.legs.read();
        legs[dest_leg].send(BusEventSource::Direct(self.leg_index), safe, msg)
    }
}

impl<ChannelId, MSG: Clone, const STACK_SIZE: usize> BusSendMultiFeature<MSG>
    for BusWorker<ChannelId, MSG, STACK_SIZE>
{
    fn broadcast(&self, safe: bool, msg: MSG) {
        let legs = self.legs.read();
        for leg in &*legs {
            let _ = leg.send(BusEventSource::Broadcast(self.leg_index), safe, msg.clone());
        }
    }
}

impl<ChannelId: Debug + Copy + Hash + PartialEq + Eq, MSG: Clone, const STACK_SIZE: usize>
    BusPubSubFeature<ChannelId, MSG> for BusWorker<ChannelId, MSG, STACK_SIZE>
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

    fn publish(&self, channel: ChannelId, safe: bool, msg: MSG) {
        let legs = self.legs.read();
        let channels = self.channels.read();
        if let Some(entry) = channels.get(&channel) {
            for &leg_index in entry {
                let _ = legs[leg_index].send(
                    BusEventSource::Channel(self.leg_index, channel),
                    safe,
                    msg.clone(),
                );
            }
        }
    }
}
