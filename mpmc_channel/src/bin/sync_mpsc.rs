use core::num::Wrapping;
use embassy_executor::Spawner;
use embassy_time::Timer;
use log::info;
use mpsc_channel::{HasAddress, Receiver, ReceiverApi, SyncChannel, SyncSender};
use static_cell::StaticCell;

const NUM_PRODUCERS: usize = 5;
const CHANNEL_CAPACITY: usize = NUM_PRODUCERS;
const NUM_CONSUMERS: usize = 1;

static mut BUFS: [[u8; 50]; CHANNEL_CAPACITY] = [[0; 50]; CHANNEL_CAPACITY];

#[derive(Default)]
struct Mpdu {
    buffer: &'static mut [u8],
}

impl HasAddress<()> for Mpdu {
    fn get_address(&self) -> () {}
}

type SyncMpscChannel = SyncChannel<(), Mpdu, CHANNEL_CAPACITY, NUM_CONSUMERS>;
type SyncMpscSender = SyncSender<'static, (), Mpdu, CHANNEL_CAPACITY, NUM_CONSUMERS>;
type MpscReceiver = Receiver<'static, (), Mpdu, CHANNEL_CAPACITY, NUM_CONSUMERS, SyncMpscChannel>;

#[embassy_executor::task(pool_size = NUM_PRODUCERS)]
async fn producer(id: u8, mut sender: SyncMpscSender) {
    let mut counter = Wrapping(0u8);
    loop {
        let now = counter.0;

        if let Some(slot) = sender.try_allocate_msg_slot() {
            let slot_id = slot.get_id();
            info!("producer {id}: sending {now} over {slot_id}");

            // Safety: We have a dedicated buffer per slot and the channel
            //         ensures that it will be released before we gain back
            //         control.
            let buffer = unsafe { &mut BUFS[slot.get_id() as usize] };
            buffer[0] = id;
            buffer[1] = slot_id;
            buffer[2] = now;

            let msg = Mpdu { buffer };
            sender.send_msg(slot, msg);

            counter += 1;
            Timer::after_millis(100).await;
        } else {
            // Spinning: Fairness is not guaranteed. Synchronous clients
            // requiring fairness need some kind of out-of-band co-ordination
            // among themselves.
            Timer::after_millis(10).await;
        }
    }
}

#[embassy_executor::task]
async fn consumer(receiver: MpscReceiver) {
    let mut cons_slot = receiver.try_allocate_cons_slot().unwrap();
    loop {
        receiver
            .receive(&mut cons_slot, (), |msg| async {
                let buffer = msg.buffer;
                // Long delivery delay to demonstrate backpressure.
                Timer::after_millis(1000).await;
                info!(
                    "producer {}: received {} over {}",
                    buffer[0], buffer[2], buffer[1]
                );
            })
            .await;
    }
}

fn mpdu_channel() -> &'static SyncMpscChannel {
    // In the synchronous case, there is no backlog.
    static CHANNEL: StaticCell<SyncMpscChannel> = StaticCell::new();
    CHANNEL.init(SyncChannel::new())
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
    spawner.spawn(consumer(channel.get_receiver())).unwrap();
}
