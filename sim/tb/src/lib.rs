//! Testbench support for the pico-gs2 GPU.
//!
//! The tests in `tests/` drive the cycle-accurate simulators that
//! `sinter build` generates from `hw/` (the `gpu-sim` crate for the whole
//! core, and one crate per `#[module]`), and check them against the
//! reference models here and the chip models in `models/`.

#![deny(unsafe_code)]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(clippy::all))]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(clippy::pedantic))]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(missing_docs))]
// Until 1.0.0, allow dead code while the testbench grows.
#![allow(dead_code)]

mod sdram;
mod tmds;

pub use sdram::{SdramBus, FLOATING};
pub use tmds::{decode, Encoder, Symbol, CONTROL_TOKENS};
