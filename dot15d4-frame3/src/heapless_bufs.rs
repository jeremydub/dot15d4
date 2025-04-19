#![allow(non_camel_case_types)]

pub const NUM_RX_BUFFERS: usize = 4;
pub const NUM_TX_BUFFERS: usize = 4;

use heapless::{
    box_pool,
    pool::boxed::{Box, BoxBlock},
};
use static_cell::ConstStaticCell;

use crate::mpdu::IMM_ACK_BUF_LEN;

// TODO: move to driver
// TODO: wrap in macro
box_pool!(IMM_ACK_BUFFER_POOL: [u8; IMM_ACK_BUF_LEN]);
static IMM_ACK_BUFFERS: ConstStaticCell<[BoxBlock<[u8; IMM_ACK_BUF_LEN]>; NUM_RX_BUFFERS]> = {
    const IMM_ACK_BUFFER: BoxBlock<[u8; IMM_ACK_BUF_LEN]> = BoxBlock::new();
    ConstStaticCell::new([IMM_ACK_BUFFER; NUM_RX_BUFFERS])
};

fn init_imm_ack_buffers() {
    for imm_ack_buffer in IMM_ACK_BUFFERS.take() {
        IMM_ACK_BUFFER_POOL.manage(imm_ack_buffer);
    }
}

pub fn init_bufs() {
    init_imm_ack_buffers();
}

// TODO: make async
pub fn imm_ack_buffer() -> Box<IMM_ACK_BUFFER_POOL> {
    IMM_ACK_BUFFER_POOL
        .alloc([0; IMM_ACK_BUF_LEN])
        .expect("ACK buffer")
}
