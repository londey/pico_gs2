//! Cycle-level model of the Winbond W9825G6KH SDRAM (4 banks x 8192 rows x
//! 512 columns x 16 bits, 32 MB), for testbenches that drive a memory
//! controller's pins.
//!
//! [`Sdram::clock`] is one rising edge of the SDRAM clock: it samples the
//! command, address, DQM and DQ pins the controller drives, and returns what
//! the chip drives on DQ until the next edge. Every command is checked
//! against the datasheet (`models/reference/sdram/w9825g6kh_a04.pdf`,
//! section 9.5, and the power-up sequence in section 7.1); each breach is
//! recorded as a [`Violation`] instead of being silently tolerated, so a test
//! fails on a controller that only works because the model is forgiving.
//!
//! Not modelled (each is reported as a violation if used): power-down and
//! self refresh (CKE low), auto-precharge, interleaved bursts, and full-page
//! bursts with BURST STOP.

#![deny(unsafe_code)]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(clippy::all))]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(clippy::pedantic))]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(missing_docs))]
// Until 1.0.0, allow dead code while the model grows.
#![allow(dead_code)]

mod sdram;
mod timing;

pub use sdram::{Dq, Pins, Sdram, Violation, BANKS, COLUMNS, ROWS};
pub use timing::Timing;
