use std::{collections::HashMap, fmt::Debug, hash::Hash};

#[derive(Debug)]
pub struct BusLocalHub<Owner, ChannelId: Hash + PartialEq + Eq> {
    channels: HashMap<ChannelId, Vec<Owner>>,
}

impl<Owner, ChannelId: Hash + PartialEq + Eq> Default for BusLocalHub<Owner, ChannelId> {
    fn default() -> Self {
        Self {
            channels: HashMap::new(),
        }
    }
}

impl<Owner: Clone + Debug + PartialEq, ChannelId: Debug + Clone + Copy + Hash + PartialEq + Eq>
    BusLocalHub<Owner, ChannelId>
{
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
        log::info!("channels: {:?}", self.channels);
        if let Some(entry) = self.channels.get_mut(&channel) {
            log::info!("remove owner {:?} with list {:?}", owner, entry);
            if let Some(pos) = entry.iter().position(|x| *x == owner) {
                log::info!("remove owner {:?}", owner);
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
    pub fn get_subscribers(&self, channel: ChannelId) -> Option<Vec<Owner>> {
        self.channels.get(&channel).cloned()
    }
}

#[cfg(test)]
mod tests {
    use crate::group_owner_type;

    use super::*;

    group_owner_type!(DemoOwner);

    #[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
    enum Channel {
        A,
        B,
        C,
    }

    #[test]
    fn test_subscribe_unsubscribe() {
        let mut hub: BusLocalHub<DemoOwner, Channel> = BusLocalHub::default();
        let owner1 = DemoOwner(1);
        let owner2 = DemoOwner(2);

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
        let mut hub: BusLocalHub<DemoOwner, Channel> = BusLocalHub::default();
        let owner1 = DemoOwner(1);
        let owner2 = DemoOwner(2);

        hub.subscribe(owner1, Channel::A);
        hub.subscribe(owner1, Channel::B);
        hub.subscribe(owner2, Channel::A);

        assert_eq!(hub.get_subscribers(Channel::A), Some(vec![owner1, owner2]));
        assert_eq!(hub.get_subscribers(Channel::B), Some(vec![owner1]));
        assert_eq!(hub.get_subscribers(Channel::C), None);
    }
}
