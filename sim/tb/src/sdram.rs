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
/// The board's SDRAM clock lags the core clock by 270 degrees
/// (`board/top.sv`), so a word the chip drives after its edge in one core
/// cycle reaches the FPGA's DQ input in time for the core clock edge that
/// ends the next. The bus models that as one cycle of delay: [`dq_in`] in a
/// cycle is what the chip drove in the previous one.
///
/// [`dq_in`]: SdramBus::dq_in
pub struct SdramBus {
    /// The chip.
    pub sdram: Sdram,

    /// DQ at the FPGA this cycle.
    dq_in: u16,
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
            dq_in: FLOATING,
        }
    }

    /// DQ as the FPGA samples it this cycle.
    ///
    /// # Returns
    ///
    /// The word on the FPGA's DQ input.
    pub fn dq_in(&self) -> u16 {
        self.dq_in
    }

    /// The chip's clock edge in this cycle: it samples the controller's
    /// pins, and what it drives reaches the FPGA next cycle.
    ///
    /// # Arguments
    ///
    /// * `pins` - The controller's pins this cycle.
    pub fn clock(&mut self, pins: &Pins) {
        let chip = self.sdram.clock(pins);
        let other = pins.dq.unwrap_or(FLOATING);
        self.dq_in = (chip.value & chip.driven) | (other & !chip.driven);
    }
}

impl Default for SdramBus {
    fn default() -> Self {
        Self::new()
    }
}
