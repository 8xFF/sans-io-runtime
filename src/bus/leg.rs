use parking_lot::Mutex;
use std::sync::Arc;

use crate::{backend::Awaker, collections::DynamicDeque};

/// Represents the identifier of a channel.
#[derive(Debug, PartialEq, Eq, Clone)]
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

struct QueueInternal<ChannelId, MSG, const STATIC_SIZE: usize> {
    awaker: Option<Arc<dyn Awaker>>,
    queue: DynamicDeque<(BusEventSource<ChannelId>, MSG), STATIC_SIZE>,
}

impl<ChannelId, MSG, const STATIC_SIZE: usize> Default
    for QueueInternal<ChannelId, MSG, STATIC_SIZE>
{
    fn default() -> Self {
        Self {
            awaker: None,
            queue: DynamicDeque::default(),
        }
    }
}

impl<ChannelId, MSG, const STATIC_SIZE: usize> QueueInternal<ChannelId, MSG, STATIC_SIZE> {
    pub fn push_safe(&mut self, source: BusEventSource<ChannelId>, msg: MSG) -> usize {
        self.queue.push_back((source, msg));
        let after = self.queue.len();
        if after == 1 {
            if let Some(awaker) = self.awaker.as_ref() {
                awaker.awake();
            }
        }
        after
    }

    pub fn push_generic(
        &mut self,
        safe: bool,
        source: BusEventSource<ChannelId>,
        msg: MSG,
    ) -> Result<usize, ()> {
        if safe {
            self.queue.push_back((source, msg));
        } else {
            self.queue.push_back_stack((source, msg)).map_err(|_| ())?;
        }
        let after = self.queue.len();
        if after == 1 {
            if let Some(awaker) = self.awaker.as_ref() {
                awaker.awake();
            }
        }
        Ok(after)
    }

    pub fn pop_front(&mut self) -> Option<(BusEventSource<ChannelId>, MSG)> {
        self.queue.pop_front()
    }
}

#[derive(Default)]
struct SharedBusQueue<ChannelId, MSG, const STATIC_SIZE: usize> {
    queue: Arc<Mutex<QueueInternal<ChannelId, MSG, STATIC_SIZE>>>,
}

impl<ChannelId, MSG, const STATIC_SIZE: usize> Clone
    for SharedBusQueue<ChannelId, MSG, STATIC_SIZE>
{
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
        }
    }
}

impl<ChannelId, MSG, const STATIC_SIZE: usize> SharedBusQueue<ChannelId, MSG, STATIC_SIZE> {
    fn send_safe(&self, source: BusEventSource<ChannelId>, msg: MSG) -> usize {
        self.queue.lock().push_safe(source, msg)
    }

    fn send(
        &self,
        source: BusEventSource<ChannelId>,
        safe: bool,
        msg: MSG,
    ) -> Result<usize, BusLegSenderErr> {
        self.queue
            .lock()
            .push_generic(safe, source, msg)
            .map_err(|_| BusLegSenderErr::ChannelFull)
    }

    fn recv(&self) -> Option<(BusEventSource<ChannelId>, MSG)> {
        self.queue.lock().pop_front()
    }

    fn set_awaker(&self, awaker: Arc<dyn Awaker>) {
        self.queue.lock().awaker = Some(awaker);
    }
}

/// A sender for a bus leg.
///
/// This struct is used to send messages through a bus leg. It holds a queue of messages
/// that are waiting to be processed by the bus leg.
pub struct BusLegSender<ChannelId, MSG, const STATIC_SIZE: usize> {
    queue: SharedBusQueue<ChannelId, MSG, STATIC_SIZE>,
}

impl<ChannelId, MSG, const STATIC_SIZE: usize> Clone for BusLegSender<ChannelId, MSG, STATIC_SIZE> {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
        }
    }
}

impl<ChannelId, MSG, const STATIC_SIZE: usize> BusLegSender<ChannelId, MSG, STATIC_SIZE> {
    /// Sends a message through the bus leg with safe, if stack full, it will append to heap.
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
    pub fn send_safe(&self, source: BusEventSource<ChannelId>, msg: MSG) -> usize {
        self.queue.send_safe(source, msg)
    }

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
        self.queue.send(source, safe, msg)
    }
}

/// A receiver for a bus leg.
///
/// This struct is used to receive messages from a bus leg. It holds a queue of messages
/// that have been sent to the bus leg.
pub struct BusLegReceiver<ChannelId, MSG, const STATIC_SIZE: usize> {
    queue: SharedBusQueue<ChannelId, MSG, STATIC_SIZE>,
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
        self.queue.recv()
    }

    pub fn set_awaker(&self, awaker: Arc<dyn Awaker>) {
        self.queue.set_awaker(awaker);
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
    let queue = SharedBusQueue {
        queue: Default::default(),
    };
    let sender = BusLegSender {
        queue: queue.clone(),
    };
    let receiver = BusLegReceiver { queue };
    (sender, receiver)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_and_recv() {
        // Create a bus leg
        let (sender, receiver) = create_bus_leg::<u32, String, 10>();

        // Send a message
        let source = BusEventSource::Channel(0, 42);
        let msg = "Hello, world!".to_string();
        let result = sender.send(source.clone(), true, msg.clone());
        assert_eq!(result, Ok(1));

        // Receive the message
        let received = receiver.recv();
        assert_eq!(received, Some((source, msg)));
    }

    #[test]
    fn test_send_channel_full() {
        // Create a bus leg with a static size of 1
        let (sender, receiver) = create_bus_leg::<u32, &'static str, 1>();
        let source = BusEventSource::Channel(0, 42);

        // Send the first message

        let result = sender.send(source.clone(), false, "Message 1");
        assert_eq!(result, Ok(1));

        // Send the second message (should fail)
        let result = sender.send(source.clone(), false, "Message 2");
        assert_eq!(result, Err(BusLegSenderErr::ChannelFull));

        // Send the second message as safe (should not fail)
        let result = sender.send(source.clone(), true, "Message 3");
        assert_eq!(result, Ok(2));

        assert_eq!(receiver.recv(), Some((source.clone(), "Message 1")));
        assert_eq!(receiver.recv(), Some((source, "Message 3")));
        assert_eq!(receiver.recv(), None);
    }

    #[test]
    fn test_recv_empty_queue() {
        // Create a bus leg
        let (_, receiver) = create_bus_leg::<u32, String, 10>();

        // Receive a message from an empty queue
        let received = receiver.recv();
        assert_eq!(received, None);
    }
}
