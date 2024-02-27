use parking_lot::Mutex;
use std::{collections::VecDeque, sync::Arc};

const BUS_CHANNEL_PRE_ALOC: usize = 64;
const BUS_CHANNEL_SIZE_LIMIT: usize = 1024;

/// Represents the identifier of a channel.
#[derive(Debug, PartialEq, Eq)]
pub enum BusEventSource<ChannelId> {
    Channel(usize, ChannelId),
    Broadcast(usize),
    Direct(usize),
    External,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BusLegSenderErr {
    ChannelFull,
}

#[derive(Debug)]
/// A sender for a bus leg.
///
/// This struct is used to send messages through a bus leg. It holds a queue of messages
/// that are waiting to be processed by the bus leg.
pub struct BusLegSender<ChannelId, MSG> {
    queue: Arc<Mutex<VecDeque<(BusEventSource<ChannelId>, MSG)>>>,
}

impl<ChannelId, MSG> Clone for BusLegSender<ChannelId, MSG> {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
        }
    }
}

impl<ChannelId, MSG> BusLegSender<ChannelId, MSG> {
    /// Sends a message through the bus leg.
    ///
    /// # Arguments
    ///
    /// * `source` - The source of the bus event.
    /// * `msg` - The message to be sent.
    ///
    /// # Returns
    ///
    /// Returns `Ok(len)` if the message is successfully sent, where `len` is the new length of the queue.
    /// Returns `Err(ChannelFull)` if the queue is already full and the message cannot be sent.
    pub fn send(
        &self,
        source: BusEventSource<ChannelId>,
        msg: MSG,
    ) -> Result<usize, BusLegSenderErr> {
        let mut queue = self.queue.lock();
        if queue.len() >= BUS_CHANNEL_SIZE_LIMIT {
            return Err(BusLegSenderErr::ChannelFull);
        }
        queue.push_back((source, msg));
        Ok(queue.len())
    }
}

/// A receiver for a bus leg.
///
/// This struct is used to receive messages from a bus leg. It holds a queue of messages
/// that have been sent to the bus leg.
pub struct BusLegReceiver<ChannelId, MSG> {
    queue: Arc<Mutex<VecDeque<(BusEventSource<ChannelId>, MSG)>>>,
}

impl<ChannelId, MSG> BusLegReceiver<ChannelId, MSG> {
    /// Receives a message from the bus leg.
    ///
    /// # Returns
    ///
    /// Returns `Some((source, msg))` if there is a message in the queue, where `source` is the source of the bus event
    /// and `msg` is the received message.
    /// Returns `None` if the queue is empty.
    pub fn recv(&self) -> Option<(BusEventSource<ChannelId>, MSG)> {
        self.queue.lock().pop_front()
    }
}

/// Creates a pair of sender and receiver for a bus leg.
///
/// # Returns
///
/// Returns a tuple `(sender, receiver)` where `sender` is a `BusLegSender` and `receiver` is a `BusLegReceiver`.
pub fn create_bus_leg<ChannelId, MSG>(
) -> (BusLegSender<ChannelId, MSG>, BusLegReceiver<ChannelId, MSG>) {
    let queue = Arc::new(Mutex::new(VecDeque::with_capacity(BUS_CHANNEL_PRE_ALOC)));
    let sender = BusLegSender {
        queue: queue.clone(),
    };
    let receiver = BusLegReceiver { queue };
    (sender, receiver)
}
