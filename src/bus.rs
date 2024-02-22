use std::{collections::HashMap, ops::Deref, sync::Arc};

mod leg;
pub use leg::*;
use parking_lot::RwLock;

#[derive(Hash, PartialEq, Eq, Debug, Clone, Copy)]
pub struct BusChannelId(pub u64);

impl Deref for BusChannelId {
    type Target = u64;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub enum BusEvent<MSG: Clone> {
    ChannelSubscribe(BusChannelId),
    ChannelUnsubscribe(BusChannelId),
    ChannelPublish(BusChannelId, MSG),
    Broadcast(MSG),
    Direct(usize, MSG),
}

#[derive(Debug, Clone)]
pub struct BusSystemBuilder<MSG: Clone> {
    legs: Vec<BusLegSender<MSG>>,
}

impl<MSG: Clone> Default for BusSystemBuilder<MSG> {
    fn default() -> Self {
        Self { legs: Vec::new() }
    }
}

impl<MSG: Clone> BusSystemBuilder<MSG> {
    pub fn new_leg(&mut self) -> BusLegReceiver<MSG> {
        let (sender, recv) = create_bus_leg();
        self.legs.push(sender);
        recv
    }

    pub fn build(self) -> BusSystem<MSG> {
        BusSystem {
            legs: self.legs,
            channels: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BusSystem<MSG: Clone> {
    legs: Vec<BusLegSender<MSG>>,
    channels: Arc<RwLock<HashMap<BusChannelId, Vec<usize>>>>,
}

impl<MSG: Clone> BusSystem<MSG> {
    pub fn on_event(&self, leg_index: usize, event: BusEvent<MSG>) {
        match event {
            BusEvent::ChannelSubscribe(channel) => {
                let mut channels = self.channels.write();
                let entry = channels.entry(channel).or_insert_with(Vec::new);
                if entry.contains(&leg_index) {
                    return;
                }
                entry.push(leg_index);
            }
            BusEvent::ChannelUnsubscribe(channel) => {
                let mut channels = self.channels.write();
                if let Some(entry) = channels.get_mut(&channel) {
                    if let Some(index) = entry.iter().position(|x| *x == leg_index) {
                        entry.swap_remove(index);
                    }
                }
            }
            BusEvent::ChannelPublish(channel, msg) => {
                let channels = self.channels.read();
                if let Some(entry) = channels.get(&channel) {
                    for &leg_index in entry {
                        let _ =
                            self.legs[leg_index].send(BusLegEvent::Channel(channel, msg.clone()));
                    }
                }
            }
            BusEvent::Broadcast(msg) => {
                for leg in &self.legs {
                    let _ = leg.send(BusLegEvent::Broadcast(msg.clone()));
                }
            }
            BusEvent::Direct(leg_index, msg) => {
                let _ = self.legs[leg_index].send(BusLegEvent::Direct(msg.clone()));
            }
        }
    }
}
