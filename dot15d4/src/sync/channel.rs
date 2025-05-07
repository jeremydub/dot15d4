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
use core::array::from_fn;
use core::cell::RefCell;
use core::future::{poll_fn, Future};
use core::task::{Poll, Waker};
use heapless::Deque;

/// A non-cloneable token representing an allocated message slot: Produced by
/// allocating a slot and consumed by committing to send a message across the
/// channel. Guarantees immediate bandwidth on the channel for a single message.
#[must_use = "Must be returned to the channel to unblock the message slot."]
pub struct MsgSlotToken(u8);

impl MsgSlotToken {
    pub fn get_id(&self) -> u8 {
        self.0
    }
}

/// A bounded queue for sending messages from multiple asynchronous tasks to a
/// single receiving task with backpressure.
///
/// The channel will buffer messages up to the guaranteed capacity and will then
/// be able to backlog a limited number of additional requests from further
/// tasks while they are waiting for a message slot to become available. Trying
/// to schedule waiting tasks beyond the capacity of the backlog will cause the
/// queue to panic.
///
/// More specifically: Given `PRODUCERS` as the number of independent tasks that
/// are accessing the queue in parallel and `CAPACITY` as the number of messages
/// that can be handled concurrently, the `BACKLOG` parameter needs to be set to
/// `PRODUCERS - CAPACITY` for panic-free queue operation.
///
/// Messages will be delivered to the receiver in the same order as they were
/// sent.
pub struct Channel<Message, const CAPACITY: usize, const BACKLOG: usize>
where
    BitsImpl<CAPACITY>: Bits,
{
    inner: RefCell<State<Message, CAPACITY, BACKLOG>>,
}

impl<Message, const CAPACITY: usize, const BACKLOG: usize> Channel<Message, CAPACITY, BACKLOG>
where
    BitsImpl<CAPACITY>: Bits,
{
    /// Initialize a new [`Channel`].
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(State::new()),
        }
    }

    /// Creates a [`Sender`] and [`Receiver`] from an existing channel.
    ///
    /// Further Senders can be created through [`Sender::borrow`].
    pub fn split(
        &mut self,
    ) -> (
        Sender<'_, Message, CAPACITY, BACKLOG>,
        Receiver<'_, Message, CAPACITY, BACKLOG>,
    ) {
        (Sender { channel: self }, Receiver { channel: self })
    }
}

/// Send-only access to a [`Channel`].
pub struct Sender<'a, Message, const CAPACITY: usize, const BACKLOG: usize>
where
    BitsImpl<CAPACITY>: Bits,
{
    channel: &'a Channel<Message, CAPACITY, BACKLOG>,
}

impl<'a, Message, const CAPACITY: usize, const BACKLOG: usize>
    Sender<'a, Message, CAPACITY, BACKLOG>
where
    BitsImpl<CAPACITY>: Bits,
{
    /// Creates one further [`Sender`] over the same channel.
    pub fn borrow(&self) -> Sender<'a, Message, CAPACITY, BACKLOG> {
        Sender {
            channel: self.channel,
        }
    }

    /// Tries to allocate a message slot.
    ///
    /// Changes the state of any allocated slot from available to allocated.
    pub fn try_allocate_msg_slot(&mut self) -> Option<MsgSlotToken> {
        match self.channel.inner.borrow_mut().allocate_msg_slot() {
            Some(slot) => Some(MsgSlotToken(slot)),
            None => None,
        }
    }

    /// Waits until a message slot becomes available and blocks it's capacity
    /// for later use by the client.
    ///
    /// Changes the state of the allocated slot from available to allocated.
    pub fn allocate_msg_slot(
        &mut self,
    ) -> impl Future<Output = MsgSlotToken> + use<'_, 'a, Message, CAPACITY, BACKLOG> {
        poll_fn(move |cx| {
            let state = &mut self.channel.inner.borrow_mut();
            match state.allocate_msg_slot() {
                Some(slot) => Poll::Ready(MsgSlotToken(slot)),
                None => {
                    state
                        .backlog
                        .push_front(cx.waker().clone())
                        .expect("backlog full");
                    Poll::Pending
                }
            }
        })
    }

    /// Synchronously sends the given message over a previously allocated slot
    /// (i.e. makes it "pending" on the receiver side).
    ///
    /// The method returns immediately ("fire and forget"). The message has not
    /// necessarily been delivered yet.
    ///
    /// Changes the state of the allocated slot from available to pending.
    ///
    /// Note: This operation cannot fail.
    pub fn send_msg(&mut self, slot: MsgSlotToken, msg: Message) {
        let MsgSlotToken(slot) = slot;
        self.send_msg_internal(slot, msg);
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
        slot: MsgSlotToken,
        msg: Message,
    ) -> impl Future<Output = ()> + use<'_, 'a, Message, CAPACITY, BACKLOG> {
        let MsgSlotToken(slot) = slot;

        self.send_msg_internal(slot, msg);

        poll_fn(move |cx| {
            let state = &mut self.channel.inner.borrow_mut();

            let delivery_state = &mut state.delivery_state[slot as usize];
            if let DeliveryState::Delivered = delivery_state {
                *delivery_state = DeliveryState::NotSent;

                state.release(slot);

                Poll::Ready(())
            } else {
                *delivery_state = DeliveryState::Sent(cx.waker().clone());
                Poll::Pending
            }
        })
    }

    fn send_msg_internal(&mut self, slot: u8, msg: Message) {
        let state = &mut self.channel.inner.borrow_mut();
        debug_assert!(!state.is_available(slot));
        debug_assert!(state.msg_slots[slot as usize].is_none());

        // Safety: A slot can never be allocated and pending at the same time.
        //         We're guaranteed exclusive access to the slot right now and
        //         may write to it. An available slot must be empty.
        state.msg_slots[slot as usize] = Some(msg);

        state.send(slot);
    }

    /// Convenience method that allocates a message slot, sends the given
    /// message as soon as a slot becomes available and then waits until the
    /// message has been delivered.
    pub async fn send(&mut self, msg: Message) -> () {
        let slot = self.allocate_msg_slot().await;
        self.send_msg_and_wait(slot, msg).await;
    }
}

/// Receive-only access to a [`Channel`].
pub struct Receiver<'a, Message, const CAPACITY: usize, const BACKLOG: usize>
where
    BitsImpl<CAPACITY>: Bits,
{
    channel: &'a Channel<Message, CAPACITY, BACKLOG>,
}

impl<'a, Message, const CAPACITY: usize, const BACKLOG: usize>
    Receiver<'a, Message, CAPACITY, BACKLOG>
where
    BitsImpl<CAPACITY>: Bits,
{
    /// Wait for a message and return it together with a slot number for
    /// asynchronous consumption.
    ///
    /// Changes the state of the allocated slot from pending to receiving.
    pub fn wait_for_msg(
        &mut self,
    ) -> impl Future<Output = (MsgSlotToken, Message)> + use<'a, Message, CAPACITY, BACKLOG> {
        poll_fn(|cx| {
            let state = &mut *self.channel.inner.borrow_mut();
            match state.receive() {
                Some(slot) => {
                    // Safety: A slot can never be allocated and pending at
                    //         the same time. We're guaranteed exclusive access
                    //         to the slot right now and may write to it. A
                    //         pending slot must have been allocated and set.
                    let msg = state.msg_slots[slot as usize].take().unwrap();
                    Poll::Ready((MsgSlotToken(slot), msg))
                }
                None => {
                    debug_assert!(state.pending_waker.is_none());
                    state.pending_waker = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        })
    }

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
    pub fn received(&mut self, slot: MsgSlotToken) {
        let MsgSlotToken(slot) = slot;
        self.channel.inner.borrow_mut().delivered(slot)
    }

    /// Convenience method over `wait_for_msg()` and `receive_done()`.
    pub async fn receive<R, Fut: Future<Output = R>, F: FnOnce(Message) -> Fut>(
        &mut self,
        f: F,
    ) -> R {
        let (slot, msg) = self.wait_for_msg().await;
        let result = f(msg).await;
        self.received(slot);
        result
    }
}

enum DeliveryState {
    NotSent,
    Sent(Waker),
    Delivered,
}

/// CAPACITY defines the number of available message slots.
/// BACKLOG defines the number of excess tasks that may be kept waiting until
/// message slots become available.
///
/// The lifecycle of an individual slot:
/// - available
/// - allocated
/// - pending
/// - receiving
/// - released (i.e. available again)
struct State<Message, const CAPACITY: usize, const BACKLOG: usize>
where
    BitsImpl<CAPACITY>: Bits,
{
    /// Pre-allocated message slots. Messages are expected to be small.
    /// Large resources linked to messages SHALL be kept in separately allocated
    /// buffers.
    msg_slots: [Option<Message>; CAPACITY],

    /// A bitmap that manages message slots: 0 - in use, 1 - available
    available: Bitmap<CAPACITY>,

    /// Contains the list of pending slots in the order they became pending.
    // Safety: This is a non-synchronized queue. Unfortunately we cannot use the
    //         synchronized [`heapless::spsc::Queue`] for now as its capacity
    //         is N-1 which unnecessarily complicates the required const generic
    //         arguments.
    pending: Deque<u8, CAPACITY>,

    /// Woken when any slot becomes pending.
    pending_waker: Option<Waker>,

    /// Woken when the message in a given slot has been delivered.
    delivery_state: [DeliveryState; CAPACITY],

    /// Contains the list of tasks waiting for a message slot in the order they
    /// started waiting.
    ///
    /// The tasks are woken in the order they started waiting as soon as a slot
    /// becomes available.
    backlog: Deque<Waker, BACKLOG>,
}

// CAPACITY is the intended capacity of the channel. BACKLOG is the number of
// producers that may wait for a slot to become available.
///
/// Safety: None of the methods are idempotent. They must not be called from
///         call-sites prone to spurious wake-ups (e.g. the pending branch of a
///         poll function).
impl<Message, const CAPACITY: usize, const BACKLOG: usize> State<Message, CAPACITY, BACKLOG>
where
    BitsImpl<CAPACITY>: Bits,
{
    fn new() -> Self {
        assert!(CAPACITY <= u8::MAX as _, "capacity > 256");
        State {
            msg_slots: from_fn(|_| None),
            available: Bitmap::mask(CAPACITY),
            pending: Deque::new(),
            pending_waker: None,
            delivery_state: from_fn(|_| DeliveryState::NotSent),
            backlog: Deque::new(),
        }
    }

    /// Guarantees unique access to the returned slot in the queue.
    fn allocate_msg_slot(&mut self) -> Option<u8> {
        match self.available.first_index() {
            None => None,
            Some(slot) => {
                self.delivery_state[slot] = DeliveryState::NotSent;
                self.available.set(slot, false);
                Some(slot as _)
            }
        }
    }

    /// Notify the receiver that a slot is pending. No-op if the slot is already pending.
    fn send(&mut self, slot: u8) {
        // Safety: Slot must be reserved (= not available) but not yet pending.
        debug_assert!(!self.is_available(slot));

        // Safety: The queue has dedicated capacity for all slots.
        self.pending.push_front(slot).unwrap();
        self.pending_waker.take().map(|waker| waker.wake());
    }

    /// Receive a pending slot - if any. Reception occurs in the order that
    /// messages became pending.
    fn receive(&mut self) -> Option<u8> {
        self.pending.pop_back()
    }

    /// Return a slot to the list of available slots.
    ///
    // Safety: The slot must still be reserved (= not available) but no longer
    //         pending.
    fn delivered(&mut self, slot: u8) {
        // Signal to the sending task, that a the message in this slot has
        // been fully received. A waker may or may not have been registered,
        // depending on the interface being used by the sender.
        if let DeliveryState::Sent(waker) = core::mem::replace(
            &mut self.delivery_state[slot as usize],
            DeliveryState::Delivered,
        ) {
            waker.wake();
        } else {
            self.release(slot);
        }
    }

    fn release(&mut self, slot: u8) {
        let was_available = self.available.set(slot as _, true);
        debug_assert!(!was_available);

        // Signal to the next task waiting for slots (if any), that a slot is
        // now available.
        self.backlog.pop_back().map(|waker| waker.wake());
    }

    fn is_available(&mut self, slot: u8) -> bool {
        self.available.get(slot as _)
    }
}
