//! Reference TMDS encoder and decoder, written directly from the DVI 1.0
//! specification (section 3.2.2, "TMDS encoding algorithm"), independently
//! of the Sinter implementation in `hw/video.snt`.

/// The four control tokens sent during blanking, indexed by `(C1 << 1) | C0`.
///
/// DVI 1.0, section 3.2.2: control data is encoded as one of four
/// transition-rich 10-bit characters. Bits are listed most significant
/// first; the least significant bit is sent first.
pub const CONTROL_TOKENS: [u16; 4] = [
    0b11_0101_0100,
    0b00_1010_1011,
    0b01_0101_0100,
    0b10_1010_1011,
];

/// A decoded TMDS symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    /// A data byte, sent while display enable is high.
    Data(u8),

    /// A control token, sent while display enable is low; the value is
    /// `(C1 << 1) | C0`.
    Control(u8),
}

/// A TMDS encoder for one channel, carrying the running disparity between
/// symbols.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Encoder {
    /// Running disparity: ones minus zeros sent since the last blanking
    /// period (the specification's `Cnt(t-1)`).
    disparity: i32,
}

impl Encoder {
    /// Creates an encoder with zero running disparity.
    ///
    /// # Returns
    ///
    /// A new encoder.
    pub fn new() -> Self {
        Self::default()
    }

    /// The running disparity after the symbols encoded so far.
    ///
    /// # Returns
    ///
    /// Ones minus zeros sent since the last control period.
    pub fn disparity(&self) -> i32 {
        self.disparity
    }

    /// Encodes one pixel clock's worth of input into a 10-bit symbol.
    ///
    /// # Arguments
    ///
    /// * `data` - The pixel data byte, used while `de` is high.
    /// * `ctrl` - `(C1 << 1) | C0`, used while `de` is low.
    /// * `de` - Display enable.
    ///
    /// # Returns
    ///
    /// The 10-bit symbol, least significant bit sent first.
    pub fn encode(&mut self, data: u8, ctrl: u8, de: bool) -> u16 {
        if !de {
            self.disparity = 0;
            return CONTROL_TOKENS[usize::from(ctrl & 0b11)];
        }

        // Stage 1: transition minimisation into q_m[8:0].
        let d = |i: u32| (data >> i) & 1 == 1;
        let ones_d = data.count_ones();
        let use_xnor = ones_d > 4 || (ones_d == 4 && !d(0));
        let mut q_m = [false; 9];
        q_m[0] = d(0);
        for i in 1..8 {
            let x = q_m[i - 1] ^ d(i as u32);
            q_m[i] = if use_xnor { !x } else { x };
        }
        q_m[8] = !use_xnor;

        // Stage 2: DC balance.
        let ones = q_m[..8].iter().filter(|&&b| b).count() as i32;
        let zeros = 8 - ones;
        let low: u16 = (0..8).map(|i| u16::from(q_m[i]) << i).sum();
        let q_m8 = u16::from(q_m[8]);
        let cnt = self.disparity;

        let (symbol, next) = if cnt == 0 || ones == zeros {
            let bits = if q_m[8] { low } else { !low & 0xFF };
            let next = if q_m[8] {
                cnt + ones - zeros
            } else {
                cnt + zeros - ones
            };
            ((u16::from(!q_m[8]) << 9) | (q_m8 << 8) | bits, next)
        } else if (cnt > 0 && ones > zeros) || (cnt < 0 && zeros > ones) {
            let next = cnt + 2 * i32::from(q_m[8]) + zeros - ones;
            ((1 << 9) | (q_m8 << 8) | (!low & 0xFF), next)
        } else {
            let next = cnt - 2 * i32::from(!q_m[8]) + ones - zeros;
            ((q_m8 << 8) | low, next)
        };

        self.disparity = next;
        symbol
    }
}

/// Decodes a 10-bit TMDS symbol, as a display's receiver would.
///
/// # Arguments
///
/// * `symbol` - The 10-bit symbol, least significant bit sent first.
///
/// # Returns
///
/// The control token's `(C1 << 1) | C0` if the symbol is one, otherwise the
/// data byte it carries.
pub fn decode(symbol: u16) -> Symbol {
    if let Some(ctrl) = CONTROL_TOKENS.iter().position(|&t| t == symbol) {
        return Symbol::Control(ctrl as u8);
    }

    let q = if symbol & (1 << 9) != 0 {
        !symbol & 0xFF
    } else {
        symbol & 0xFF
    };
    let xor = symbol & (1 << 8) != 0;
    let mut data = q & 1;
    for i in 1..8 {
        let bit = ((q >> i) ^ (q >> (i - 1))) & 1;
        data |= (if xor { bit } else { bit ^ 1 }) << i;
    }
    Symbol::Data(data as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every byte survives encode then decode, from every running disparity
    /// the first few bytes can reach.
    #[test]
    fn data_round_trips() {
        for first in 0..=255_u8 {
            let mut enc = Encoder::new();
            enc.encode(first, 0, true);
            for data in 0..=255_u8 {
                let mut e = enc;
                assert_eq!(decode(e.encode(data, 0, true)), Symbol::Data(data));
            }
        }
    }

    /// Each control token decodes to its control value and resets the
    /// running disparity.
    #[test]
    fn control_round_trips() {
        for ctrl in 0..4_u8 {
            let mut enc = Encoder::new();
            enc.encode(0x00, 0, true);
            assert_ne!(enc.disparity(), 0);
            assert_eq!(decode(enc.encode(0, ctrl, false)), Symbol::Control(ctrl));
            assert_eq!(enc.disparity(), 0);
        }
    }
}
