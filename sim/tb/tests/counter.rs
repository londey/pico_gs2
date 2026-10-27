//! Placeholder twin test: the LEDs show the top bits of a free-running
//! counter in `hw/gpu.snt`.

use tb::{Gpu, Inputs};

/// Cycles per LED count step: the LEDs are bits 29..25 of the counter.
const CYCLES_PER_STEP: u32 = 1 << 25;

/// Checks that the LEDs start dark and step once every 2^25 cycles.
#[test]
fn leds_count_up() {
    let mut sim = Gpu::new();

    for step in 0..3_u8 {
        for _ in 0..CYCLES_PER_STEP {
            assert_eq!(sim.step(&Inputs::default()).out_led, step);
        }
    }
}
