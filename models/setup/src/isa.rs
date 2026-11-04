//! The setup engine's microcode: operations, address spaces, micro-words
//! and their encoding, and the pipeline timing that the scheduler, the
//! interpreter and the Sinter engine share.
//!
//! Each cycle the sequencer issues one micro-word: one multiplier (MUL)
//! slot operation and one ALU slot operation. The word is two ROM words at
//! the same address, [`MulWord`] (38 bits) and [`AluWord`] (33 bits), each
//! under Sinter's 64-bit limit.

/// Vertex field codes, read space `role << 4 | field` (ALU only).
pub mod field {
    /// Screen x.
    pub const X: u8 = 0;
    /// Screen y.
    pub const Y: u8 = 1;
    /// The first attribute (R); attribute `i` of [`Attr::ALL`](crate::Attr)
    /// is field `ATTR + i`.
    pub const ATTR: u8 = 2;
}

/// Read-space bases (7-bit addresses).
pub mod read {
    /// Vertex fields by role: `VERTEX + (role << 4 | field)`, roles 0 to 2.
    pub const VERTEX: u8 = 0x00;
    /// Constants ([`Constant`](super::Constant)).
    pub const CONST: u8 = 0x30;
    /// The ALU's temporary bank, 32 registers.
    pub const ALU_BANK: u8 = 0x40;
    /// The multiplier's temporary bank, 32 registers.
    pub const MUL_BANK: u8 = 0x60;
}

/// Registers in each temporary bank.
pub const BANK_SIZE: u8 = 32;

/// Output slots (6-bit), each written by one slot only.
pub mod out {
    /// Edge values E01, E12, E20 (MUL).
    pub const E: u8 = 0x00;
    /// Gradients: attribute `i`'s d/dx at `DX + 2i`, d/dy at `DX + 2i + 1`
    /// (MUL).
    pub const DX: u8 = 0x03;
    /// Edge coefficients A01, B01, A12, B12, A20, B20 (ALU).
    pub const AB: u8 = 0x20;
    /// Bounding box: xs, ys, xmax, ymax (ALU).
    pub const BOX: u8 = 0x26;
    /// Start values: attribute `i` at `START + i` (ALU).
    pub const START: u8 = 0x2A;
    /// Output slots in all.
    pub const SLOTS: usize = 64;
}

/// The constants the ALU can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Constant {
    /// 0.
    Zero,
    /// Half a pixel, as a coordinate count.
    Half,
    /// The last pixel centre's x, as a coordinate count.
    XLimit,
    /// The last pixel centre's y.
    YLimit,
    /// 2.0 as a `Q2.33` count.
    Two,
}

impl Constant {
    /// Every constant, in address order.
    pub const ALL: [Constant; 5] = [
        Constant::Zero,
        Constant::Half,
        Constant::XLimit,
        Constant::YLimit,
        Constant::Two,
    ];

    /// The constant's value, as a 36-bit count.
    pub fn value(self) -> i64 {
        match self {
            Constant::Zero => 0,
            Constant::Half => 8,
            Constant::XLimit => crate::X_LIMIT.to_count(),
            Constant::YLimit => crate::Y_LIMIT.to_count(),
            Constant::Two => 2 << 33,
        }
    }

    /// The constant's read address.
    pub fn addr(self) -> u8 {
        read::CONST + self as u8
    }
}

/// Multiplier slot operations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MulOp {
    /// Nothing.
    #[default]
    Nop = 0,
    /// Start a sum: `acc = a * b`.
    Mul = 1,
    /// `acc += a * b`, continuing the previous MUL-slot operation's sum.
    Mac = 2,
    /// `acc -= a * b`, continuing the previous MUL-slot operation's sum.
    Msub = 3,
}

/// ALU slot operations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AluOp {
    /// Nothing.
    #[default]
    Nop = 0,
    /// `a`.
    Mov = 1,
    /// `a + b`.
    Add = 2,
    /// `a - b`.
    Sub = 3,
    /// `-a`.
    Neg = 4,
    /// The smaller of `a` and `b`.
    Min = 5,
    /// The larger of `a` and `b`.
    Max = 6,
    /// The start coordinate: `min(a, b)` clipped to 0 and snapped down to
    /// the 8-pixel grid.
    Snap = 7,
    /// Normalise a positive area `a`: the `Q2.33` mantissa, and `E`.
    Lzc = 8,
    /// The reciprocal seed for mantissa `a`, from the seed ROM.
    Rcpld = 9,
    /// Set the flags from `a`: N (negative), Z (zero). Writes nothing else.
    Tst = 10,
    /// A start value: `a + (b << 16)`, `b` an attribute in native units.
    Addv = 11,
}

/// What the sequencer does after a micro-word.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Seq {
    /// The next word.
    #[default]
    Next = 0,
    /// The area check: abort if Z or culled, restart flipped if N, else
    /// carry on.
    Check = 1,
    /// The program is done: commit the outputs.
    End = 2,
}

/// A destination: an optional temporary in the slot's own bank, and an
/// optional output slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Dest {
    /// Register index in the slot's bank, 0 to 31.
    pub temp: Option<u8>,

    /// Output slot, 0 to 63.
    pub out: Option<u8>,
}

/// The multiplier slot's half of a micro-word.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MulWord {
    /// The operation.
    pub op: MulOp,

    /// The 36-bit operand's read address (a temporary).
    pub a: u8,

    /// The 18-bit operand's read address (a temporary).
    pub b: u8,

    /// Where the sum goes. Only the last operation of a sum writes.
    pub dest: Dest,

    /// The post-shifter's constant right shift.
    pub shift: u8,

    /// Also shift right by `E`.
    pub by_e: bool,

    /// Round half up.
    pub round: bool,

    /// Saturate to 36 bits (else wrap).
    pub sat: bool,
}

/// The ALU slot's half of a micro-word, with the sequencer field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AluWord {
    /// The operation.
    pub op: AluOp,

    /// The first operand's read address.
    pub a: u8,

    /// The second operand's read address.
    pub b: u8,

    /// Where the result goes.
    pub dest: Dest,

    /// What the sequencer does next.
    pub seq: Seq,
}

/// One micro-word.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MicroWord {
    /// The multiplier slot.
    pub mul: MulWord,

    /// The ALU slot and the sequencer.
    pub alu: AluWord,
}

/// Cycles from an ALU operation's issue to the first issue that can read
/// its result: operands one cycle after issue, the write the cycle after.
pub const ALU_LATENCY: usize = 2;

/// Cycles from an RCPLD's issue to the first issue that can read its
/// result: one more cycle, through the seed ROM.
pub const RCPLD_LATENCY: usize = 3;

/// Cycles from a MUL-slot issue to the first issue that can read its sum:
/// operands, multiply, product register, accumulate, shift and write.
pub const MUL_LATENCY: usize = 6;

/// Cycles from a TST's issue to a CHECK that can see its flags.
pub const TST_LATENCY: usize = 2;

/// The multiplier pipeline stage, counted from issue, in which the
/// post-shifter uses `E`.
pub const MUL_SHIFT_STAGE: usize = 5;

/// Packs a destination: temp valid and index (6 bits), out valid and slot
/// (7 bits).
fn pack_dest(d: Dest) -> (u64, u64) {
    let temp = d.temp.map_or(0, |t| 0x20 | u64::from(t));
    let out = d.out.map_or(0, |o| 0x40 | u64::from(o));
    (temp, out)
}

/// Unpacks [`pack_dest`].
fn unpack_dest(temp: u64, out: u64) -> Dest {
    Dest {
        temp: (temp & 0x20 != 0).then(|| u8::try_from(temp & 0x1F).expect("5 bits")),
        out: (out & 0x40 != 0).then(|| u8::try_from(out & 0x3F).expect("6 bits")),
    }
}

/// Field `bits` wide at `lo`, as a `u8`.
fn field8(w: u64, lo: u32, bits: u32) -> u8 {
    u8::try_from((w >> lo) & ((1 << bits) - 1)).expect("at most 8 bits")
}

impl MulWord {
    /// Bits in an encoded word.
    pub const BITS: u32 = 38;

    /// Encodes the word: op [37:36], a [35:29], b [28:22], temp [21:16],
    /// out [15:9], shift [8:3], `by_e` [2], round [1], sat [0].
    pub fn encode(&self) -> u64 {
        let (temp, out) = pack_dest(self.dest);
        (self.op as u64) << 36
            | u64::from(self.a) << 29
            | u64::from(self.b) << 22
            | temp << 16
            | out << 9
            | u64::from(self.shift) << 3
            | u64::from(self.by_e) << 2
            | u64::from(self.round) << 1
            | u64::from(self.sat)
    }

    /// Decodes [`MulWord::encode`].
    pub fn decode(w: u64) -> MulWord {
        MulWord {
            op: match (w >> 36) & 3 {
                1 => MulOp::Mul,
                2 => MulOp::Mac,
                3 => MulOp::Msub,
                _ => MulOp::Nop,
            },
            a: field8(w, 29, 7),
            b: field8(w, 22, 7),
            dest: unpack_dest((w >> 16) & 0x3F, (w >> 9) & 0x7F),
            shift: field8(w, 3, 6),
            by_e: (w >> 2) & 1 == 1,
            round: (w >> 1) & 1 == 1,
            sat: w & 1 == 1,
        }
    }
}

impl AluWord {
    /// Bits in an encoded word.
    pub const BITS: u32 = 33;

    /// Encodes the word: op [32:29], a [28:22], b [21:15], temp [14:9],
    /// out [8:2], seq [1:0].
    pub fn encode(&self) -> u64 {
        let (temp, out) = pack_dest(self.dest);
        (self.op as u64) << 29
            | u64::from(self.a) << 22
            | u64::from(self.b) << 15
            | temp << 9
            | out << 2
            | self.seq as u64
    }

    /// Decodes [`AluWord::encode`].
    pub fn decode(w: u64) -> AluWord {
        const OPS: [AluOp; 12] = [
            AluOp::Nop,
            AluOp::Mov,
            AluOp::Add,
            AluOp::Sub,
            AluOp::Neg,
            AluOp::Min,
            AluOp::Max,
            AluOp::Snap,
            AluOp::Lzc,
            AluOp::Rcpld,
            AluOp::Tst,
            AluOp::Addv,
        ];
        AluWord {
            op: OPS
                .get(usize::try_from((w >> 29) & 0xF).expect("4 bits"))
                .copied()
                .unwrap_or_default(),
            a: field8(w, 22, 7),
            b: field8(w, 15, 7),
            dest: unpack_dest((w >> 9) & 0x3F, (w >> 2) & 0x7F),
            seq: match w & 3 {
                1 => Seq::Check,
                2 => Seq::End,
                _ => Seq::Next,
            },
        }
    }
}
