//! Reference models of the pico-gs2 triangle setup engine
//! (`docs/design/setup-engine-brief.md`, ADR 0001).
//!
//! Setup turns a triangle's three vertices into what traversal steps: three
//! edge functions, a bounding box, and for each attribute a value at the
//! start point and its gradients across and down the screen.
//!
//! - [`float::setup`] is the `f64` reference: the brief's §4.2 mathematics
//!   with no rounding to speak of, used to measure the error of the fixed
//!   point version.
//! - [`setup`] is the bit-exact fixed-point version, written with
//!   [`qfixed`] types so that each value's format is part of its type. The
//!   microcoded engine, and its interpreter, must match it bit for bit.
//!
//! Number formats use Sinter's (and `qfixed`'s) Q notation: `Q<I, F>` has
//! `I` integer bits **including** the sign and `F` fraction bits (ADR 0007).

#![deny(unsafe_code)]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(clippy::all))]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(clippy::pedantic))]
#![cfg_attr(all(not(debug_assertions), not(test)), deny(missing_docs))]
// Pedantic lints that don't suit this crate (the standard set).
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
// Until 1.0.0, allow dead code while the model grows.
#![allow(dead_code)]

pub mod float;
mod formats;
mod reciprocal;
mod setup;
mod triangle;

pub use formats::{
    post_shift, AttrDelta, Coord, Delta, Iter, Mantissa, Recip, Seed, Wide, ACC_BITS, K, PRE_SHIFT,
    REG_BITS,
};
pub use reciprocal::{normalise, reciprocal, seed, Normalised, SEEDS};
pub use setup::{setup, Edge, Plane, Reject, Setup, X_LIMIT, Y_LIMIT};
pub use triangle::{Attr, AttrSet, CullMode, Triangle, Vertex};
