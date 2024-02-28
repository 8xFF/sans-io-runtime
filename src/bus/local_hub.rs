use std::{collections::HashMap, hash::Hash};

use crate::Owner;

#[derive(Debug)]
pub struct BusLocalHub<ChannelId: Hash + PartialEq + Eq> {
    channels: HashMap<ChannelId, Vec<Owner>>,
}

impl<ChannelId: Hash + PartialEq + Eq> Default for BusLocalHub<ChannelId> {
    fn default() -> Self {
        Self {
            channels: HashMap::new(),
        }
    }
}

impl<ChannelId: Clone + Copy + Hash + PartialEq + Eq> BusLocalHub<ChannelId> {
    /// subscribe to a channel. if it is first time subscription, return true; else return false
    pub fn subscribe(&mut self, owner: Owner, channel: ChannelId) -> bool {
        let entry = self.channels.entry(channel).or_default();
        if entry.contains(&owner) {
            false
        } else {
            entry.push(owner);
            entry.len() == 1
        }
    }

    /// unsubscribe from a channel. if it is last time unsubscription, return true; else return false
    pub fn unsubscribe(&mut self, owner: Owner, channel: ChannelId) -> bool {
        if let Some(entry) = self.channels.get_mut(&channel) {
            if let Some(pos) = entry.iter().position(|x| *x == owner) {
                entry.swap_remove(pos);
                if entry.is_empty() {
                    self.channels.remove(&channel);
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        }
    }

    /// get all subscribers of a channel
    pub fn get_subscribers(&self, channel: ChannelId) -> Option<&[Owner]> {
        self.channels.get(&channel).map(|x| x.as_slice())
    }

    /// remove owner from all channels
    pub fn remove_owner(&mut self, owner: Owner) {
        let mut removed_channels = vec![];
        for (channel, entry) in self.channels.iter_mut() {
            if let Some(pos) = entry.iter().position(|x| x == &owner) {
                entry.swap_remove(pos);
            }
            if entry.is_empty() {
                removed_channels.push(*channel);
            }
        }
        for channel in removed_channels {
            self.channels.remove(&channel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
    enum Channel {
        A,
        B,
        C,
    }

    #[test]
    fn test_subscribe_unsubscribe() {
        let mut hub: BusLocalHub<Channel> = BusLocalHub::default();
        let owner1 = Owner::worker(1);
        let owner2 = Owner::worker(2);

        assert_eq!(hub.subscribe(owner1, Channel::A), true);
        assert_eq!(hub.subscribe(owner2, Channel::A), false);
        assert_eq!(hub.subscribe(owner1, Channel::B), true);
        assert_eq!(hub.subscribe(owner1, Channel::C), true);

        assert_eq!(hub.unsubscribe(owner1, Channel::A), false);
        assert_eq!(hub.unsubscribe(owner2, Channel::A), true);
        assert_eq!(hub.unsubscribe(owner1, Channel::B), true);
        assert_eq!(hub.unsubscribe(owner1, Channel::C), true);
    }

    #[test]
    fn test_get_subscribers() {
        let mut hub: BusLocalHub<Channel> = BusLocalHub::default();
        let owner1 = Owner::worker(1);
        let owner2 = Owner::worker(2);

        hub.subscribe(owner1, Channel::A);
        hub.subscribe(owner1, Channel::B);
        hub.subscribe(owner2, Channel::A);

        assert_eq!(hub.get_subscribers(Channel::A), Some(&[owner1, owner2][..]));
        assert_eq!(hub.get_subscribers(Channel::B), Some(&[owner1][..]));
        assert_eq!(hub.get_subscribers(Channel::C), None);
    }

    #[test]
    fn test_remove_owner() {
        let mut hub: BusLocalHub<Channel> = BusLocalHub::default();
        let owner1 = Owner::worker(1);
        let owner2 = Owner::worker(2);

        hub.subscribe(owner1, Channel::A);
        hub.subscribe(owner1, Channel::B);
        hub.subscribe(owner2, Channel::A);

        hub.remove_owner(owner1);

        assert_eq!(hub.get_subscribers(Channel::A), Some(&[owner2][..]));
        assert_eq!(hub.get_subscribers(Channel::B), None);
    }
}
