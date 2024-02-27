use parking_lot::Mutex;
use std::sync::Arc;

use crate::collections::DynamicDeque;

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

/// A sender for a bus leg.
///
/// This struct is used to send messages through a bus leg. It holds a queue of messages
/// that are waiting to be processed by the bus leg.
pub struct BusLegSender<ChannelId, MSG, const STATIC_SIZE: usize> {
    queue: Arc<Mutex<DynamicDeque<(BusEventSource<ChannelId>, MSG), STATIC_SIZE>>>,
}

impl<ChannelId, MSG, const STATIC_SIZE: usize> Clone for BusLegSender<ChannelId, MSG, STATIC_SIZE> {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
        }
    }
}

impl<ChannelId, MSG, const STATIC_SIZE: usize> BusLegSender<ChannelId, MSG, STATIC_SIZE> {
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
        safe: bool,
        msg: MSG,
    ) -> Result<usize, BusLegSenderErr> {
        let mut queue = self.queue.lock();
        queue
            .push_back(safe, (source, msg))
            .map_err(|_| BusLegSenderErr::ChannelFull)?;
        Ok(queue.len())
    }
}

/// A receiver for a bus leg.
///
/// This struct is used to receive messages from a bus leg. It holds a queue of messages
/// that have been sent to the bus leg.
pub struct BusLegReceiver<ChannelId, MSG, const STATIC_SIZE: usize> {
    queue: Arc<Mutex<DynamicDeque<(BusEventSource<ChannelId>, MSG), STATIC_SIZE>>>,
}

impl<ChannelId, MSG, const STATIC_SIZE: usize> BusLegReceiver<ChannelId, MSG, STATIC_SIZE> {
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
pub fn create_bus_leg<ChannelId, MSG, const STATIC_SIZE: usize>() -> (
    BusLegSender<ChannelId, MSG, STATIC_SIZE>,
    BusLegReceiver<ChannelId, MSG, STATIC_SIZE>,
) {
    let queue = Arc::new(Mutex::new(DynamicDeque::new()));
    let sender = BusLegSender {
        queue: queue.clone(),
    };
    let receiver = BusLegReceiver { queue };
    (sender, receiver)
}
