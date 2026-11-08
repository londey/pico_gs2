//! Testbench support for the pico-gs2 GPU.
//!
//! The tests in `tests/` drive the cycle-accurate simulators that
//! `sinter build` generates from `hw/` (the `gpu-sim` crate for the whole
//! core, and one crate per `#[module]`), and check them against the
//! reference models here and the chip models in `models/`. [`Bench`] puts
//! the whole core on a bench with the SDRAM model and a [`Host`] on its USB
//! UART.

#![deny(unsafe_code)]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(clippy::all))]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(clippy::pedantic))]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(missing_docs))]
// Until 1.0.0, allow dead code while the testbench grows.
#![allow(dead_code)]

mod bench;
mod host;
mod sdram;
mod tmds;
mod uart;

pub use bench::{
    differing, rgb888, screen, tiled, Bench, Shown, CLOCK_HZ, FB_HEIGHT, FB_WIDTH, FRAME, H_ACTIVE,
    H_SYNC_END, H_SYNC_START, H_TOTAL, LED_FILLED, LED_SDRAM_READY, LED_UNDERFLOW, SCALE,
    SYMBOL_LATENCY, V_ACTIVE, V_SYNC_END, V_SYNC_START, V_TOTAL,
};
pub use host::Host;
pub use sdram::{SdramBus, FLOATING};
pub use tmds::{decode, Encoder, Symbol, CONTROL_TOKENS};
pub use uart::{UartRx, UartTx, BIT_CYCLES};
