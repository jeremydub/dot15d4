use core::num::Wrapping;
use embassy_executor::Spawner;
use embassy_time::Timer;
use log::*;
use mpsc_channel::{AsyncChannel, AsyncSender, HasAddress, Receiver, ReceiverApi};
use static_cell::StaticCell;

const NUM_PRODUCERS: usize = 10;
const CHANNEL_CAPACITY: usize = 5;
const BACKLOG: usize = if NUM_PRODUCERS > CHANNEL_CAPACITY {
    NUM_PRODUCERS - CHANNEL_CAPACITY
} else {
    1
};
const NUM_CONSUMERS: usize = NUM_PRODUCERS / 2;

// TODO: Simulate safe, limited buffer pool with heapless Box allocator.
static mut BUFS: [[u8; 50]; NUM_PRODUCERS] = [[0; 50]; NUM_PRODUCERS];

#[derive(Clone, Default)]
struct Address(u8);

impl PartialEq for Address {
    fn eq(&self, other: &Self) -> bool {
        // Simulate "1-bit subnet" matching so that we can define per-subnet
        // consumers.
        self.0 >> 1 == other.0 >> 1
    }
}

#[derive(Default)]
struct Mpdu {
    address: Address,
    buffer: &'static mut [u8],
}

impl HasAddress<Address> for Mpdu {
    fn get_address(&self) -> Address {
        self.address.clone()
    }
}

type AsyncMpmcChannel = AsyncChannel<Address, Mpdu, CHANNEL_CAPACITY, BACKLOG, NUM_CONSUMERS>;
type AsyncMpmcSender =
    AsyncSender<'static, Address, Mpdu, CHANNEL_CAPACITY, BACKLOG, NUM_CONSUMERS>;
type MpmcReceiver =
    Receiver<'static, Address, Mpdu, CHANNEL_CAPACITY, NUM_CONSUMERS, AsyncMpmcChannel>;

#[embassy_executor::task(pool_size = NUM_PRODUCERS)]
async fn producer(id: u8, mut sender: AsyncMpmcSender) {
    let mut counter = Wrapping(0u8);
    loop {
        let now = counter.0;

        info!("sending: from-addr {id} to-subnet {} value {now}", id & !1);

        // Safety: We have a dedicated buffer per producer and the receiver's
        //         delivery semantics ensures that it will be released before we
        //         gain back control.
        let buffer = unsafe { &mut BUFS[id as usize] };
        buffer[0] = id;
        buffer[1] = now;

        let msg = Mpdu {
            address: Address(id),
            buffer,
        };
        sender.send(msg).await;

        info!(
            "delivered: from-addr {id} to-subnet {} value {now}",
            id & !1
        );

        counter += 1;

        Timer::after_millis(100 - id as u64).await;
    }
}

#[embassy_executor::task(pool_size = NUM_CONSUMERS)]
async fn consumer(id: u8, receiver: MpmcReceiver) {
    let subnet = id << 1;
    let mut cons_slot = receiver.try_allocate_cons_slot().unwrap();
    loop {
        receiver
            .receive(&mut cons_slot, Address(subnet), |msg| async {
                let buffer = msg.buffer;
                // Long delivery delay to demonstrate backpressure.
                Timer::after_millis(1000).await;
                info!(
                    "receiving: from-addr {} from_subnet {} to-subnet {subnet}: value {}",
                    buffer[0],
                    buffer[0] & !1,
                    buffer[1]
                );
            })
            .await;
    }
}

fn mpdu_channel() -> &'static AsyncMpmcChannel {
    static CHANNEL: StaticCell<AsyncMpmcChannel> = StaticCell::new();
    CHANNEL.init(AsyncMpmcChannel::new())
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .format_timestamp_nanos()
        .init();

    let channel = mpdu_channel();
    for id in 0..NUM_PRODUCERS {
        spawner
            .spawn(producer(id as u8, channel.get_sender()))
            .unwrap();
    }
    for id in 0..NUM_CONSUMERS {
        spawner
            .spawn(consumer(id as u8, channel.get_receiver()))
            .unwrap();
    }
}
