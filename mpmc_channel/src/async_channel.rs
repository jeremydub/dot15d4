use core::{
    array::from_fn,
    cell::{RefCell, RefMut},
    future::{poll_fn, Future},
    marker::PhantomData,
    task::Poll,
    task::Waker,
};

use bitmaps::{Bits, BitsImpl};
use heapless::Deque;

use crate::{
    ConsSlotToken, HasAddress, InternalReceiverApi, Receiver, ReceiverApi, State, TxMsgSlotToken,
};

/// An asynchronous bounded queue for sending messages from multiple
/// asynchronous tasks to a single receiving task with backpressure.
///
/// The channel will buffer messages up to the guaranteed capacity and will then
/// be able to backlog a limited number of additional requests from further
/// tasks while they are waiting for a message slot to become available. Trying
/// to schedule waiting tasks beyond the capacity of the backlog will cause the
/// queue to panic.
///
/// More specifically: Given `PRODUCERS` as the number of independent tasks that
/// are accessing the queue in parallel and `MESSAGES` as the number of messages
/// that can be handled concurrently, the `BACKLOG` parameter needs to be set to
/// `PRODUCERS - MESSAGES` for panic-free queue operation.
///
/// Messages will be delivered to the receiver in the same order as they were
/// sent.
pub struct AsyncChannel<
    Address: PartialEq + Clone,
    Message: HasAddress<Address>,
    const MESSAGES: usize,
    const BACKLOG: usize,
    const CONSUMERS: usize,
> where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    state: RefCell<State<Address, Message, MESSAGES, CONSUMERS>>,
    async_state: RefCell<AsyncState<MESSAGES, BACKLOG>>,
}

impl<
        Address: PartialEq + Clone,
        Message: HasAddress<Address>,
        const MESSAGES: usize,
        const BACKLOG: usize,
        const CONSUMERS: usize,
    > AsyncChannel<Address, Message, MESSAGES, BACKLOG, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    /// Initialize a new [`Channel`].
    pub fn new() -> Self {
        Self {
            state: RefCell::new(State::new()),
            async_state: RefCell::new(AsyncState::new()),
        }
    }

    /// Returns an additional [`AsyncSender`] attached to the channel.
    pub fn get_sender(&self) -> AsyncSender<'_, Address, Message, MESSAGES, BACKLOG, CONSUMERS> {
        AsyncSender { channel: self }
    }

    /// Returns an additional [`Receiver`] attached to the channel.
    pub fn get_receiver(&self) -> Receiver<'_, Address, Message, MESSAGES, CONSUMERS, Self> {
        Receiver {
            channel: self,
            address: PhantomData,
            message: PhantomData,
        }
    }
}

enum DeliveryState {
    NotSent,
    Sent(Waker),
    Delivered,
}

/// Asynchronous send-only access to a [`Channel`].
pub struct AsyncSender<
    'a,
    Address: PartialEq + Clone,
    Message: HasAddress<Address>,
    const MESSAGES: usize,
    const BACKLOG: usize,
    const CONSUMERS: usize,
> where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    channel: &'a AsyncChannel<Address, Message, MESSAGES, BACKLOG, CONSUMERS>,
}

impl<
        'a,
        Address: PartialEq + Clone,
        Message: HasAddress<Address>,
        const MESSAGES: usize,
        const BACKLOG: usize,
        const CONSUMERS: usize,
    > AsyncSender<'a, Address, Message, MESSAGES, BACKLOG, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    /// Waits until a message slot becomes available and blocks it's capacity
    /// for later use by the client.
    ///
    /// Changes the state of the allocated slot from available to allocated.
    pub fn allocate_msg_slot(
        &mut self,
    ) -> impl Future<Output = TxMsgSlotToken> + use<'a, Address, Message, MESSAGES, BACKLOG, CONSUMERS>
    {
        poll_fn(|cx| {
            let state = &mut self.channel.state.borrow_mut();
            let async_state = &mut self.channel.async_state.borrow_mut();
            match state.allocate_msg_slot() {
                Some(slot) => {
                    async_state.msg_slot_allocated(slot);
                    Poll::Ready(TxMsgSlotToken(slot))
                }
                None => {
                    async_state.msg_slot_unavailable(cx.waker().clone());
                    Poll::Pending
                }
            }
        })
    }

    /// Sends the given message and then waits until it has been delivered and
    /// the message slot released.
    ///
    /// Changes the state of the allocated slot from available to pending and
    /// waits until it is being released (i.e. becomes available again).
    ///
    /// Note: As we hand over ownership of messages on delivery, their content
    ///       (including any references to buffers or other dependent resources)
    ///       may leak beyond the time of return of this method. You may not
    ///       assume that resources assigned to the message can be re-used
    ///       unless you implement drop handlers that allow you to prove that
    ///       dependent resources blocked by the message have actually been
    ///       released.
    pub fn send_msg_and_wait(
        &mut self,
        slot: TxMsgSlotToken,
        msg: Message,
    ) -> impl Future<Output = ()> + use<'_, 'a, Address, Message, MESSAGES, BACKLOG, CONSUMERS>
    {
        let TxMsgSlotToken(slot) = slot;
        self.channel.state.borrow_mut().send(slot, msg);

        poll_fn(move |cx| {
            let state = &mut self.channel.state.borrow_mut();
            let async_state = &mut self.channel.async_state.borrow_mut();

            let delivery_state = &mut async_state.delivery_state[slot as usize];
            if let DeliveryState::Delivered = delivery_state {
                *delivery_state = DeliveryState::NotSent;

                state.release_msg_slot(slot);
                async_state.msg_slot_now_available();

                Poll::Ready(())
            } else {
                *delivery_state = DeliveryState::Sent(cx.waker().clone());
                Poll::Pending
            }
        })
    }

    /// Convenience method that allocates a message slot, sends the given
    /// message as soon as a slot becomes available and then waits until the
    /// message has been delivered.
    pub async fn send(&mut self, msg: Message) -> () {
        let slot = self.allocate_msg_slot().await;
        self.send_msg_and_wait(slot, msg).await;
    }
}

impl<
        Address: PartialEq + Clone,
        Message: HasAddress<Address>,
        const MESSAGES: usize,
        const BACKLOG: usize,
        const CONSUMERS: usize,
    > ReceiverApi<Address, Message> for AsyncChannel<Address, Message, MESSAGES, BACKLOG, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    fn try_allocate_cons_slot(&self) -> Option<ConsSlotToken> {
        <Self as InternalReceiverApi<_, _, MESSAGES, CONSUMERS>>::try_allocate_cons_slot(&self)
    }

    fn release_cons_slot(&self, cons_slot: ConsSlotToken) {
        <Self as InternalReceiverApi<_, _, MESSAGES, CONSUMERS>>::release_cons_slot(
            &self, cons_slot,
        );
    }

    fn wait_for_msg(
        &self,
        cons_slot: &mut ConsSlotToken,
        address: Address,
    ) -> impl Future<Output = (TxMsgSlotToken, Message)> {
        <Self as InternalReceiverApi<_, _, MESSAGES, CONSUMERS>>::wait_for_msg(
            &self, cons_slot, address,
        )
    }

    fn received(&self, slot: TxMsgSlotToken) {
        let TxMsgSlotToken(slot) = slot;
        self.async_state
            .borrow_mut()
            .delivered(slot, || self.state.borrow_mut().release_msg_slot(slot));
    }
}

impl<
        Address: PartialEq + Clone,
        Message: HasAddress<Address>,
        const MESSAGES: usize,
        const BACKLOG: usize,
        const CONSUMERS: usize,
    > InternalReceiverApi<Address, Message, MESSAGES, CONSUMERS>
    for AsyncChannel<Address, Message, MESSAGES, BACKLOG, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    fn get_state(&self) -> RefMut<'_, State<Address, Message, MESSAGES, CONSUMERS>> {
        self.state.borrow_mut()
    }
}

struct AsyncState<const MESSAGES: usize, const BACKLOG: usize>
where
    BitsImpl<MESSAGES>: Bits,
{
    /// Woken when the message in a given slot has been delivered.
    delivery_state: [DeliveryState; MESSAGES],

    /// Contains the list of tasks waiting for a message slot in the order they
    /// started waiting.
    ///
    /// The tasks are woken in the order they started waiting as soon as a slot
    /// becomes available.
    backlog: Deque<Waker, BACKLOG>,
}

// MESSAGES is the intended capacity of the channel. BACKLOG is the number of
// producers that may wait for a slot to become available.
///
/// Safety: None of the methods are idempotent. They must not be called from
///         call-sites prone to spurious wake-ups (e.g. the pending branch of a
///         poll function).
impl<const MESSAGES: usize, const BACKLOG: usize> AsyncState<MESSAGES, BACKLOG>
where
    BitsImpl<MESSAGES>: Bits,
{
    fn new() -> Self {
        Self {
            delivery_state: from_fn(|_| DeliveryState::NotSent),
            backlog: Deque::new(),
        }
    }

    fn msg_slot_allocated(&mut self, slot: u8) {
        self.delivery_state[slot as usize] = DeliveryState::NotSent;
    }

    fn msg_slot_unavailable(&mut self, waker: Waker) {
        self.backlog.push_front(waker).expect("backlog full");
    }

    fn msg_slot_now_available(&mut self) {
        // Signal to the next task waiting for slots (if any), that a slot is
        // now available.
        self.backlog.pop_back().map(|waker| waker.wake());
    }

    /// Return a slot to the list of available slots.
    ///
    // Safety: The slot must still be reserved (= not available) but no longer
    //         pending.
    fn delivered<F: FnOnce()>(&mut self, slot: u8, release_slot: F) {
        // Signal to the sending task, that a the message in this slot has
        // been fully received. A waker may or may not have been registered,
        // depending on the interface being used by the sender.
        if let DeliveryState::Sent(waker) = core::mem::replace(
            &mut self.delivery_state[slot as usize],
            DeliveryState::Delivered,
        ) {
            waker.wake();
        } else {
            release_slot();
        }
    }
}
