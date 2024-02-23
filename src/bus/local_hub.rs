use std::collections::HashMap;

use crate::BusChannelId;

#[derive(Debug, Default)]
pub struct BusLocalHub<Owner> {
    channels: HashMap<BusChannelId, Vec<Owner>>,
}

impl<Owner: PartialEq + Eq + Clone + Copy> BusLocalHub<Owner> {
    /// subscribe to a channel. if it is first time subscription, return true; else return false
    pub fn subscribe(&mut self, channel: BusChannelId, owner: Owner) -> bool {
        let entry = self.channels.entry(channel).or_default();
        if entry.contains(&owner) {
            false
        } else {
            entry.push(owner);
            entry.len() == 1
        }
    }

    /// unsubscribe from a channel. if it is last time unsubscription, return true; else return false
    pub fn unsubscribe(&mut self, channel: BusChannelId, owner: Owner) -> bool {
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
    pub fn get_subscribers(&self, channel: BusChannelId) -> Option<&[Owner]> {
        self.channels.get(&channel).map(|x| x.as_slice())
    }

    /// remove owner from all channels
    pub fn remove_owner(&mut self, owner: Owner) {
        for (_, entry) in self.channels.iter_mut() {
            if let Some(pos) = entry.iter().position(|x| x == &owner) {
                entry.swap_remove(pos);
            }
        }
    }

    /// remove owner from all channels
    pub fn swap_owner(&mut self, from: Owner, to: Owner) {
        for (_, entry) in self.channels.iter_mut() {
            if let Some(pos) = entry.iter().position(|x| x == &from) {
                entry[pos] = to;
            }
        }
    }
}
