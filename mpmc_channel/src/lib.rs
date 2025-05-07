//! A bounded queue for sending messages from multiple asynchronous tasks to a
//! single receiving task with backpressure.
//!
//! The channel is not synchronized across threads. It can be used concurrently
//! by multiple producer (sender) tasks as long as both, the producers and the
//! receiver, are managed by a single executor (thread).
//!
//! This module provides a bounded channel that has a limit on the number of
//! messages that it can store. If this limit is reached, trying to send another
//! message will exert backpressure on the client.
//!
//! The channel exposes two different APIs to senders.
//!
//! 1. Synchronous API:
//!
//!   - try_allocate_msg_slot() -> Option<token>
//!     Returns a message token if a slot is available, None otherwise.
//!
//!   - send_msg(token, msg)
//!     Sends a message, guaranteed to succeed. Returns immediately without
//!     delivery feedback.
//!
//!   This is a "fire and forget" API for synchronous producers. It can be used
//!   to support smoltcp's device abstraction.
//!
//! 2. Asynchronous API:
//!
//!   - allocate_msg_slot() -> token
//!     Waits until a message token becomes available and returns it.
//!
//!   - send_msg_and_wait(token, msg)
//!     Sends the given message and waits until it was delivered to the
//!     receiver.
//!
//!   - send(msg)
//!     a convenience method over allocate_msg_slot() and send_msg_and_wait()
//!
//!   This API targets asynchronous producers, provides feedback about delivery
//!   and thereby facilitates safe resource management. It can be extended
//!   towards an asynchronous request/response communication model in the
//!   future.
//!
//! On the receiver side, a single async API is exposed:
//!
//!   - wait_for_msg() -> (token, msg)
//!     Waits until a message becomes pending. Also returns the token required
//!     to signal delivery to the sender.
//!
//!   - received(token)
//!     Releases the message slot and signals delivery to the sender.
//!
//!   - receive(msg_cb) -> result
//!     a convenience method over wait_for_msg() and received() given a closure
//!     that handles message reception

use bitmaps::{Bitmap, Bits, BitsImpl};
use core::{
    array::from_fn,
    cell::RefMut,
    future::{poll_fn, Future},
    marker::PhantomData,
    task::{Poll, Waker},
};
use heapless::Deque;

mod async_channel;
mod sync_channel;

pub use async_channel::{AsyncChannel, AsyncSender};
pub use sync_channel::{SyncChannel, SyncSender};

/// A non-cloneable token representing an allocated message slot: Produced by
/// allocating a slot and consumed by sending a message across the channel.
/// Guarantees bandwidth on the channel for a single message.
#[must_use = "Must be returned to the channel to unblock the message slot."]
pub struct TxMsgSlotToken(u8);

impl TxMsgSlotToken {
    pub fn get_id(&self) -> u8 {
        self.0
    }
}

/// A non-cloneable token representing an allocated consumer slot: Produced by
/// allocating a slot. Must be presented to receive a message from the channel.
/// Guarantees bandwidth on the channel for a single consumer.
#[must_use = "Must be presented to the channel to access the consumer slot."]
pub struct ConsSlotToken(u8);

impl ConsSlotToken {
    pub fn get_id(&self) -> u8 {
        self.0
    }
}

// TODO: Add allocator to API.
// TODO: Add return value to send() (Placed in the same message slot with a variant over request/response values).
// TODO: Add a try_receive() method to peek at the receive queue.
// TODO: Make a prioritized version of the channel for timestamped radio tasks
//       that allows awaiting changes to the queue front.

/// Receive-only access to a [`Channel`].
pub struct Receiver<
    'a,
    Address: PartialEq + Clone,
    Message: HasAddress<Address>,
    const MESSAGES: usize,
    const CONSUMERS: usize,
    Channel: InternalReceiverApi<Address, Message, MESSAGES, CONSUMERS>,
> where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    channel: &'a Channel,
    address: PhantomData<Address>,
    message: PhantomData<Message>,
}

pub trait ReceiverApi<Address: PartialEq + Clone, Message: HasAddress<Address>> {
    /// Try to allocate a consumer slot.
    ///
    /// Consumers typically acquire a slot once and then re-use it for each
    /// reception.
    fn try_allocate_cons_slot(&self) -> Option<ConsSlotToken>;

    /// Release a consumer slot.
    ///
    /// Consumers typically call this method just before the consumer task ends.
    fn release_cons_slot(&self, cons_slot: ConsSlotToken);

    /// Wait for a message matching the given address or address wildcard and
    /// return it together with a message slot for asynchronous consumption.
    ///
    /// Reception occurs in the order that messages became pending.
    ///
    /// If several receivers match a pending message only a single, arbitrary
    /// one will receive the message.
    ///
    /// Note: Other than on the producer side (which may have to wait for a
    ///       radio slot to become available) we do not allow for backpressure
    ///       on the consumer side as we currently assume that local consumers
    ///       should be fast enough to handle reception synchronously. Therefore
    ///       it is required to present an allocated consumer slot to get access
    ///       to the channel.
    ///
    /// Changes the state of any matching message slot from pending to
    /// receiving.
    fn wait_for_msg(
        &self,
        cons_slot: &mut ConsSlotToken,
        address: Address,
    ) -> impl Future<Output = (TxMsgSlotToken, Message)>;

    /// Releases the message slot for re-use and signals delivery to
    /// asynchronous senders.
    ///
    /// Defining the notion of "delivery" is up to the implementor. It can
    /// signal delivery of a message over the air, a promise that certain
    /// resources have been released, reception of a response by a peer, etc.
    ///
    /// Changes the state of the allocated slot from receiving to released
    /// (available).
    ///
    /// Note: Not calling this method will block the slot forever.
    fn received(&self, msg_slot: TxMsgSlotToken);

    /// Convenience method over `wait_for_msg()` and `receive_done()`.
    fn receive<R, Fut: Future<Output = R>, F: FnOnce(Message) -> Fut>(
        &self,
        cons_slot: &mut ConsSlotToken,
        address: Address,
        f: F,
    ) -> impl Future<Output = R> {
        async {
            let (msg_slot, msg) = self.wait_for_msg(cons_slot, address).await;
            let result = f(msg).await;
            self.received(msg_slot);
            result
        }
    }
}

/// Helper struct that allows us to remove an inner value from [`Deque`]
/// represented as the return value of [`Deque::as_mut_slices()`].
struct DequeWrapper<'a, const N: usize>(&'a mut Deque<u8, N>);

impl<'a, const N: usize> DequeWrapper<'a, N> {
    fn new(deque: &'a mut Deque<u8, N>) -> Self {
        Self(deque)
    }

    /// Moves each entry in the slice up to the (index-1)'th entry one position
    /// to the back, so that the front entry becomes empty and can be removed.
    fn remove(&mut self, index: usize) {
        let (first, second) = self.0.as_mut_slices();

        // TODO: Check whether this loop needs to be optimized.
        for i in (1..=index).rev() {
            let prev = Self::get(first, second, i - 1);
            Self::set(first, second, i, prev);
        }

        self.0.pop_front();
    }

    fn get(first: &[u8], second: &[u8], index: usize) -> u8 {
        let len_of_first = first.len();
        if index < len_of_first {
            first[index]
        } else {
            second[index - len_of_first]
        }
    }

    fn set(first: &mut [u8], second: &mut [u8], index: usize, val: u8) {
        let len_of_first = first.len();
        if index < len_of_first {
            first[index] = val;
        } else {
            second[index - len_of_first] = val;
        }
    }
}

pub trait InternalReceiverApi<
    Address: PartialEq + Clone,
    Message: HasAddress<Address>,
    const MESSAGES: usize,
    const CONSUMERS: usize,
>: ReceiverApi<Address, Message> where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    /// See [`ReceiverApi::try_allocate_cons_slot()`]
    fn try_allocate_cons_slot(&self) -> Option<ConsSlotToken> {
        self.get_state()
            .allocate_cons_slot()
            .map(|slot| ConsSlotToken(slot))
    }

    /// See [`ReceiverApi::release_cons_slot()`]
    fn release_cons_slot(&self, cons_slot: ConsSlotToken) {
        let ConsSlotToken(slot) = cons_slot;
        self.get_state().release_cons_slot(slot);
    }

    /// See [`ReceiverApi::wait_for_msg()`]
    fn wait_for_msg(
        &self,
        cons_slot: &mut ConsSlotToken, // Mutability guarantees exclusive access.
        address: Address,
    ) -> impl Future<Output = (TxMsgSlotToken, Message)> {
        poll_fn(move |cx| {
            let mut state = self.get_state();
            let ConsSlotToken(cons_slot) = *cons_slot;

            for (index, msg_slot) in state.msg_pending.iter().enumerate() {
                let msg_slot = *msg_slot;

                // Check whether this consumer listens for the pending message.
                match &state.messages[msg_slot as usize] {
                    Some(msg) => {
                        if address == msg.get_address() {
                            // Safety: A slot can never be allocated and pending at
                            //         the same time. We're guaranteed exclusive access
                            //         to the slot right now and may write to it. A
                            //         pending slot must have been allocated and set.
                            let msg = state.messages[msg_slot as usize].take().unwrap();

                            // Remove the pending message from the list.
                            DequeWrapper::new(&mut state.msg_pending).remove(index);

                            return Poll::Ready((TxMsgSlotToken(msg_slot), msg));
                        }
                    }
                    None => unreachable!(),
                }
            }

            // None of the pending messages fits the given address, so let's
            // wait for one that fits.
            debug_assert!(state.consumers[cons_slot as usize].is_none());
            state.consumers[cons_slot as usize] = Some((address.clone(), cx.waker().clone()));
            Poll::Pending
        })
    }

    /// Get the internal state of the receiving channel.
    fn get_state(&self) -> RefMut<'_, State<Address, Message, MESSAGES, CONSUMERS>>;
}

impl<
        'a,
        Address: PartialEq + Clone,
        Message: HasAddress<Address>,
        const MESSAGES: usize,
        const CONSUMERS: usize,
        Channel: InternalReceiverApi<Address, Message, MESSAGES, CONSUMERS>,
    > ReceiverApi<Address, Message> for Receiver<'a, Address, Message, MESSAGES, CONSUMERS, Channel>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    fn try_allocate_cons_slot(&self) -> Option<ConsSlotToken> {
        InternalReceiverApi::try_allocate_cons_slot(self.channel)
    }

    fn release_cons_slot(&self, cons_slot: ConsSlotToken) {
        InternalReceiverApi::release_cons_slot(self.channel, cons_slot);
    }

    fn wait_for_msg(
        &self,
        cons_slot: &mut ConsSlotToken,
        address: Address,
    ) -> impl Future<Output = (TxMsgSlotToken, Message)> {
        InternalReceiverApi::wait_for_msg(self.channel, cons_slot, address)
    }

    fn received(&self, msg_slot: TxMsgSlotToken) {
        self.channel.received(msg_slot)
    }

    fn receive<R, Fut: Future<Output = R>, F: FnOnce(Message) -> Fut>(
        &self,
        cons_slot: &mut ConsSlotToken,
        address: Address,
        f: F,
    ) -> impl Future<Output = R> {
        self.channel.receive(cons_slot, address, f)
    }
}

pub trait HasAddress<Address> {
    fn get_address(&self) -> Address;
}

/// MESSAGES defines the number of available slots for messages that may be sent
/// concurrently over the channel.
///
/// CONSUMERS defines the number of available slots for consumers that may be
/// concurrently listening for matching messages on the channel.
///
/// The lifecycle of an individual message slot:
/// - available
/// - allocated
/// - pending
/// - receiving
/// - released (i.e. available again)
pub struct State<
    Address: PartialEq + Clone,
    Message: HasAddress<Address>,
    const MESSAGES: usize,
    const CONSUMERS: usize,
> where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    /// Pre-allocated message slots. Messages are expected to be small.
    ///
    /// Large resources linked to messages SHALL be kept in separately allocated
    /// buffers.
    messages: [Option<Message>; MESSAGES],

    /// A bitmap that manages message slots: 0 - in use, 1 - available
    msg_available: Bitmap<MESSAGES>,

    /// Contains the list of pending slots in the order they became pending.
    // Safety: This is a non-synchronized queue. Unfortunately we cannot use the
    //         synchronized [`heapless::spsc::Queue`] for now as its capacity
    //         is N-1 which unnecessarily complicates the required const generic
    //         arguments.
    msg_pending: Deque<u8, MESSAGES>,

    /// When a message becomes pending, the first consumer selectable by the
    /// message's address will be woken and receives the message.
    ///
    /// The consumer address may be a wildcard address matching all messages or
    /// a well-defined selection. A corresponding [`PartialEq`] implementation
    /// must be given for the address space.
    ///
    /// Note: Currently we assume that the consumer list is short.  Therefore
    ///       iteratively O(n)-searching for matching consumers is less resource
    ///       intensive than using a hash map, binary search or similar.
    ///
    /// Note: Messages are not cloneable in general. As we hand out ownership of
    ///       messages to consumers, only a single consumer should currently be
    ///       matching per message.
    ///
    ///       This might change in the future: We may introduce an order to
    ///       consumers and re-define consumers as "filters" such that each
    ///       consumer may individually decide whether they consume a message
    ///       (OK), drop it (DROP) or pass it on to consumers further down the
    ///       chain without consuming it themselves (CONTINUE).
    consumers: [Option<(Address, Waker)>; CONSUMERS],

    /// A bitmap that manages consumer slots: 0 - in use, 1 - available
    cons_available: Bitmap<CONSUMERS>,
}

/// Safety: None of the methods are idempotent. They must not be called from
///         call-sites prone to spurious wake-ups (e.g. the pending branch of a
///         poll function).
impl<
        Address: PartialEq + Clone,
        Message: HasAddress<Address>,
        const MESSAGES: usize,
        const CONSUMERS: usize,
    > State<Address, Message, MESSAGES, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    fn new() -> Self {
        assert!(MESSAGES <= u8::MAX as _, "producers > 256");
        Self {
            messages: from_fn(|_| None),
            msg_available: Bitmap::mask(MESSAGES),
            msg_pending: Deque::new(),
            consumers: from_fn(|_| None),
            cons_available: Bitmap::mask(CONSUMERS),
        }
    }

    /// Guarantees exclusive access to the returned message slot in the channel.
    fn allocate_msg_slot(&mut self) -> Option<u8> {
        match self.msg_available.first_index() {
            None => None,
            Some(slot) => {
                self.msg_available.set(slot, false);
                Some(slot as u8)
            }
        }
    }

    /// Return a message slot to the list of available slots.
    ///
    /// Safety: The slot must still be allocated (= not available) but no longer
    ///         pending.
    fn release_msg_slot(&mut self, slot: u8) {
        let was_available = self.msg_available.set(slot as _, true);
        debug_assert!(!was_available);
    }

    /// Checks whether the message slot with the given slot id is available.
    fn is_msg_slot_available(&mut self, slot: u8) -> bool {
        self.msg_available.get(slot as _)
    }

    /// Guarantees exclusive access to the returned consumer slot in the
    /// channel.
    fn allocate_cons_slot(&mut self) -> Option<u8> {
        match self.cons_available.first_index() {
            None => None,
            Some(slot) => {
                self.cons_available.set(slot, false);
                Some(slot as u8)
            }
        }
    }

    /// Return a consumer slot to the list of available slots.
    ///
    /// Safety: The slot must still be allocated.
    fn release_cons_slot(&mut self, slot: u8) {
        let was_available = self.cons_available.set(slot as _, true);
        debug_assert!(!was_available);
    }

    /// Store the message, mark the slot as pending and notify the first
    /// matching consumer (if any) that a message is pending.
    fn send(&mut self, slot: u8, msg: Message) {
        // Safety: Slot must be reserved (= not available) but not yet pending.
        debug_assert!(!self.is_msg_slot_available(slot));
        debug_assert!(self.messages[slot as usize].is_none());

        let msg_address = msg.get_address();

        // Safety: A slot can never be allocated and pending at the same time.
        //         We're guaranteed exclusive access to the slot right now and
        //         may write to it. An available slot must be empty.
        self.messages[slot as usize] = Some(msg);

        // Safety: The queue has dedicated capacity for all slots.
        self.msg_pending.push_back(slot).unwrap();

        // Wake the first matching consumer (if any).
        for consumer in &mut self.consumers {
            if let Some((cons_address, _)) = consumer {
                if *cons_address == msg_address {
                    consumer.take().map(|(_, waker)| waker.wake());
                    break;
                }
            }
        }
    }
}
