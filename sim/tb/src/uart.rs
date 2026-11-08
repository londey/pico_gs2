//! UART line models at the host link's rate (3 Mbaud on the 75 MHz core
//! clock: 25 cycles a bit), 8N1, LSB first, one call per core cycle.

use std::collections::VecDeque;

/// Core cycles per bit.
pub const BIT_CYCLES: u32 = 25;

/// Sends bytes on a UART line.
#[derive(Clone, Debug, Default)]
pub struct UartTx {
    /// Bytes waiting to be sent.
    queue: VecDeque<u8>,

    /// The frame being sent, LSB first: start bit, 8 data bits, stop bit.
    frame: u16,

    /// Bits of the frame still to send, including the current one.
    bits: u32,

    /// Cycles left of the current bit.
    count: u32,
}

impl UartTx {
    /// Queues a byte.
    ///
    /// # Arguments
    ///
    /// * `b` - The byte.
    pub fn push(&mut self, b: u8) {
        self.queue.push_back(b);
    }

    /// Whether nothing is queued or being sent.
    ///
    /// # Returns
    ///
    /// `true` when idle.
    pub fn idle(&self) -> bool {
        self.queue.is_empty() && self.bits == 0
    }

    /// The line's level this cycle, moving on a cycle.
    ///
    /// # Returns
    ///
    /// 1 (idle, a stop bit or a 1) or 0.
    pub fn line(&mut self) -> u8 {
        if self.bits == 0 {
            let Some(b) = self.queue.pop_front() else {
                return 1;
            };
            self.frame = 1 << 9 | u16::from(b) << 1;
            self.bits = 10;
            self.count = BIT_CYCLES;
        }
        let level = u8::from(self.frame & 1 == 1);
        self.count -= 1;
        if self.count == 0 {
            self.frame >>= 1;
            self.bits -= 1;
            self.count = BIT_CYCLES;
        }
        level
    }
}

/// Receives bytes from a UART line, sampling each bit in its middle.
#[derive(Clone, Debug, Default)]
pub struct UartRx {
    /// In a frame.
    busy: bool,

    /// Cycles to the next sample.
    count: u32,

    /// The bit sampled next: 0 the start bit, 1 to 8 data, 9 stop.
    bit: u32,

    /// The data so far, LSB first.
    data: u8,

    /// Frames whose stop bit was low.
    pub framing_errors: usize,
}

impl UartRx {
    /// Takes the line's level for one cycle.
    ///
    /// # Arguments
    ///
    /// * `level` - The line, 0 or 1.
    ///
    /// # Returns
    ///
    /// A byte, if this cycle's sample completes one.
    pub fn clock(&mut self, level: u8) -> Option<u8> {
        if !self.busy {
            if level == 0 {
                self.busy = true;
                self.count = BIT_CYCLES / 2;
                self.bit = 0;
            }
            return None;
        }
        self.count -= 1;
        if self.count > 0 {
            return None;
        }
        self.count = BIT_CYCLES;
        match self.bit {
            0 if level == 1 => {
                // A glitch, not a start bit.
                self.busy = false;
                None
            }
            0 => {
                self.bit = 1;
                None
            }
            1..=8 => {
                self.data = self.data >> 1 | level << 7;
                self.bit += 1;
                None
            }
            _ => {
                self.busy = false;
                if level == 1 {
                    Some(self.data)
                } else {
                    self.framing_errors += 1;
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes sent come back, back to back.
    #[test]
    fn bytes_loop_back() {
        let mut tx = UartTx::default();
        let mut rx = UartRx::default();
        let sent = [0x00, 0xFF, 0x55, 0xA5, 0x01];
        for b in sent {
            tx.push(b);
        }
        let mut got = Vec::new();
        for _ in 0..(sent.len() as u32 + 1) * 10 * BIT_CYCLES {
            if let Some(b) = rx.clock(tx.line()) {
                got.push(b);
            }
        }
        assert_eq!(got, sent);
        assert!(tx.idle());
        assert_eq!(rx.framing_errors, 0);
    }
}
