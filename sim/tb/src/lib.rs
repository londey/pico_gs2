//! Testbench support for the pico-gs2 GPU.
//!
//! The tests in `tests/` drive the cycle-accurate simulator that
//! `sinter build` generates from `hw/gpu.snt` (the `gpu-sim` crate).

#![deny(unsafe_code)]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(clippy::all))]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(clippy::pedantic))]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(missing_docs))]
// Until 1.0.0, allow dead code while the testbench grows.
#![allow(dead_code)]

pub use gpu_sim::{Gpu, Inputs, Outputs};
