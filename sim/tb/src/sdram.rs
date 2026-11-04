//! The board between the FPGA and the SDRAM: the chip model, and the DQ
//! bus as the FPGA sees it.

use sdram_model::{Pins, Sdram, Timing};

/// SDRAM clock period on the board (75 MHz), in picoseconds.
const TCK_PS: u64 = 13_333;

/// What the FPGA reads on DQ bits nobody drives. Not 0, so that a
/// controller capturing on the wrong cycle reads something recognisable.
pub const FLOATING: u16 = 0xDEAD;

/// The SDRAM and its data bus.
///
/// What the chip drives after its clock edge in a core cycle is on the
/// FPGA's DQ input in time for the core clock edge that ends that same
/// cycle. The board showed this: with a cycle of delay modelled here (and
/// the controller capturing a cycle later to match), every word read back
/// on the board was the one after it, which until Phase 8's tiled
/// framebuffer looked like a display losing its left column.
pub struct SdramBus {
    /// The chip.
    pub sdram: Sdram,
}

impl SdramBus {
    /// The chip at power-up, with the worst-case timing of all speed grades
    /// at 75 MHz.
    ///
    /// # Returns
    ///
    /// The bus, with DQ floating.
    pub fn new() -> Self {
        Self {
            sdram: Sdram::new(Timing::any_grade(TCK_PS)),
        }
    }

    /// The chip's clock edge in this cycle: it samples the controller's
    /// pins, and what it drives is on the FPGA's DQ input this cycle.
    ///
    /// The controller's pins are all registered, so a testbench gets them
    /// for this cycle with the simulator's `eval` before it `step`s with
    /// the DQ this returns.
    ///
    /// # Arguments
    ///
    /// * `pins` - The controller's pins this cycle.
    ///
    /// # Returns
    ///
    /// The word on the FPGA's DQ input this cycle.
    pub fn clock(&mut self, pins: &Pins) -> u16 {
        let chip = self.sdram.clock(pins);
        let other = pins.dq.unwrap_or(FLOATING);
        (chip.value & chip.driven) | (other & !chip.driven)
    }
}

impl Default for SdramBus {
    fn default() -> Self {
        Self::new()
    }
}
