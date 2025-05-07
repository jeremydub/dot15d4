use core::{
    cell::{RefCell, RefMut},
    marker::PhantomData,
};

use bitmaps::{Bits, BitsImpl};

use crate::{
    ConsSlotToken, HasAddress, InternalReceiverApi, Receiver, ReceiverApi, State, TxMsgSlotToken,
};

/// A synchronous bounded queue for sending messages from multiple asynchronous
/// tasks to a single receiving task with backpressure.
///
/// The channel will buffer messages up to the guaranteed capacity. Attempts to
/// allocate further message slots will fail.
///
/// Messages will be delivered to the receiver in the same order as they were
/// sent.
pub struct SyncChannel<
    Address: PartialEq + Clone,
    Message: HasAddress<Address>,
    const MESSAGES: usize,
    const CONSUMERS: usize,
> where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    state: RefCell<State<Address, Message, MESSAGES, CONSUMERS>>,
}

impl<
        Address: PartialEq + Clone,
        Message: HasAddress<Address>,
        const MESSAGES: usize,
        const CONSUMERS: usize,
    > SyncChannel<Address, Message, MESSAGES, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    /// Initialize a new [`Channel`].
    pub fn new() -> Self {
        Self {
            state: RefCell::new(State::new()),
        }
    }

    /// Returns an additional [`SyncSender`] attached to the channel.
    pub fn get_sender(&self) -> SyncSender<'_, Address, Message, MESSAGES, CONSUMERS> {
        SyncSender { channel: self }
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

/// Synchronous send-only access to a [`Channel`].
pub struct SyncSender<
    'a,
    Address: PartialEq + Clone,
    Message: HasAddress<Address>,
    const MESSAGES: usize,
    const CONSUMERS: usize,
> where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    channel: &'a SyncChannel<Address, Message, MESSAGES, CONSUMERS>,
}

impl<
        'a,
        Address: PartialEq + Clone,
        Message: HasAddress<Address>,
        const MESSAGES: usize,
        const CONSUMERS: usize,
    > SyncSender<'a, Address, Message, MESSAGES, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    /// Tries to allocate a message slot.
    ///
    /// Changes the state of any allocated slot from available to allocated.
    pub fn try_allocate_msg_slot(&mut self) -> Option<TxMsgSlotToken> {
        match self.channel.state.borrow_mut().allocate_msg_slot() {
            Some(slot) => Some(TxMsgSlotToken(slot)),
            None => None,
        }
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
    pub fn send_msg(&mut self, slot: TxMsgSlotToken, msg: Message) {
        let TxMsgSlotToken(slot) = slot;
        self.channel.state.borrow_mut().send(slot, msg);
    }
}

impl<
        Address: PartialEq + Clone,
        Message: HasAddress<Address>,
        const MESSAGES: usize,
        const CONSUMERS: usize,
    > ReceiverApi<Address, Message> for SyncChannel<Address, Message, MESSAGES, CONSUMERS>
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
    ) -> impl std::prelude::rust_2024::Future<Output = (TxMsgSlotToken, Message)> {
        <Self as InternalReceiverApi<_, _, MESSAGES, CONSUMERS>>::wait_for_msg(
            &self, cons_slot, address,
        )
    }

    fn received(&self, slot: TxMsgSlotToken) {
        let TxMsgSlotToken(slot) = slot;
        self.state.borrow_mut().release_msg_slot(slot);
    }
}

impl<
        Address: PartialEq + Clone,
        Message: HasAddress<Address>,
        const MESSAGES: usize,
        const CONSUMERS: usize,
    > InternalReceiverApi<Address, Message, MESSAGES, CONSUMERS>
    for SyncChannel<Address, Message, MESSAGES, CONSUMERS>
where
    BitsImpl<MESSAGES>: Bits,
    BitsImpl<CONSUMERS>: Bits,
{
    fn get_state(&self) -> RefMut<'_, State<Address, Message, MESSAGES, CONSUMERS>> {
        self.state.borrow_mut()
    }
}
