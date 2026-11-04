//! The cycle-level interpreter of the microcode: the golden model the
//! Sinter setup engine is compared with, cycle by cycle.
//!
//! It runs encoded micro-words, as the ROMs hold them, with the pipeline
//! of `isa`. An operation's operands are the banks' contents at the start
//! of its issue cycle (a read in the same cycle as a write sees the old
//! word). An ALU result lands at the end of the cycle after issue, an
//! RCPLD's a cycle later. A multiplier term accumulates four cycles after
//! issue, and its sum is shifted (with `E` as it is in that cycle) and
//! lands at the end of the cycle after.

use crate::formats::{post_shift, wrap, Coord, Delta, Iter, Wide, REG_BITS};
use crate::isa::{
    out, read, AluOp, AluWord, Constant, Dest, MulOp, MulWord, Seq, BANK_SIZE, MUL_SHIFT_STAGE,
};
use crate::reciprocal::SEED_TABLE;
use crate::setup::{Edge, Plane, Reject, Setup};
use crate::triangle::{Attr, CullMode, Triangle};

/// A program in ROM form: each cycle's encoded MUL and ALU words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rom {
    /// The MUL-slot words.
    pub mul: Vec<u64>,

    /// The ALU-slot words.
    pub alu: Vec<u64>,
}

/// A memory the engine writes: each has one write port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Port {
    /// The ALU's temporary bank.
    AluBank,
    /// The multiplier's temporary bank.
    MulBank,
    /// The output slots the ALU writes.
    AluOut,
    /// The output slots the multiplier writes.
    MulOut,
}

/// One write to a bank or output slot, as the engine's write port carries
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Traced {
    /// The cycle, counted from the program's first issue, in which the
    /// write port is driven (the word lands at its end).
    pub cycle: usize,

    /// The memory.
    pub port: Port,

    /// The register or output slot.
    pub addr: u8,

    /// The 36-bit word.
    pub value: i64,
}

/// The result of running a program on one triangle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// Cycles from the first issue to the END (or the abort), inclusive.
    pub cycles: usize,

    /// The outputs, or why the triangle was discarded.
    pub result: Result<Setup, Reject>,

    /// Every bank and output write, in cycle order, until the END or the
    /// abort.
    pub trace: Vec<Traced>,
}

/// Where a write goes.
#[derive(Clone, Copy, Debug)]
enum Target {
    /// A bank register: 0 the ALU's, 1 the MUL's.
    Reg(usize, u8),
    /// An output slot.
    Out(u8),
    /// The exponent register.
    E,
    /// The flags: bit 1 N, bit 0 Z.
    Flags,
}

/// A write that lands at the end of a cycle.
#[derive(Clone, Copy, Debug)]
struct Write {
    /// The cycle at whose end it lands.
    at: usize,
    /// Where.
    target: Target,
    /// The value.
    value: i64,
}

/// A multiplier term or sum on its way down the pipeline.
#[derive(Clone, Copy, Debug)]
struct Stage {
    /// The cycle in which it reaches its next stage.
    at: usize,
    /// The word it was issued with.
    word: MulWord,
    /// The product (before accumulating) or the sum (after).
    value: i64,
}

/// The engine's state while it runs one triangle.
struct Engine<'a> {
    /// The triangle.
    tri: &'a Triangle,
    /// Physical vertex for each role.
    roles: [usize; 3],
    /// The two temporary banks: ALU, MUL.
    banks: [[i64; BANK_SIZE as usize]; 2],
    /// The output slots.
    outputs: [Option<i64>; out::SLOTS],
    /// The exponent register.
    e: i64,
    /// The flags: bit 1 N, bit 0 Z.
    flags: i64,
    /// The accumulator.
    acc: i64,
    /// Writes not yet landed.
    pending: Vec<Write>,
    /// Terms waiting to accumulate.
    products: Vec<Stage>,
    /// Sums waiting to be shifted.
    sums: Vec<Stage>,
    /// The bank and output writes that have landed.
    trace: Vec<Traced>,
}

impl Engine<'_> {
    /// The value at a read address, as things are at the start of this
    /// cycle.
    fn read(&self, addr: u8) -> i64 {
        if addr < read::CONST {
            let (role, f) = (usize::from(addr >> 4), addr & 0xF);
            let v = &self.tri[self.roles[role]];
            match f {
                0 => v.x.to_count(),
                1 => v.y.to_count(),
                f => Attr::ALL[usize::from(f - 2)].of(v),
            }
        } else if addr < read::ALU_BANK {
            Constant::ALL[usize::from(addr - read::CONST)].value()
        } else if addr < read::MUL_BANK {
            self.banks[0][usize::from(addr - read::ALU_BANK)]
        } else {
            self.banks[1][usize::from(addr - read::MUL_BANK)]
        }
    }

    /// Queues the writes of a result to its destination.
    fn write(&mut self, at: usize, bank: usize, dest: Dest, value: i64) {
        if let Some(t) = dest.temp {
            self.pending.push(Write {
                at,
                target: Target::Reg(bank, t),
                value,
            });
        }
        if let Some(o) = dest.out {
            self.pending.push(Write {
                at,
                target: Target::Out(o),
                value,
            });
        }
    }

    /// Issues an ALU word in cycle `now`.
    // Operands a and b, as in the ISA.
    #[allow(clippy::many_single_char_names)]
    fn alu(&mut self, now: usize, w: AluWord) {
        let a = self.read(w.a);
        let b = self.read(w.b);
        let r36 = |x: i64| wrap(x, REG_BITS);
        let value = match w.op {
            AluOp::Nop => return,
            AluOp::Mov => a,
            AluOp::Add => r36(a + b),
            AluOp::Sub => r36(a - b),
            AluOp::Neg => r36(-a),
            AluOp::Min => a.min(b),
            AluOp::Max => a.max(b),
            AluOp::Snap => a.min(b).max(0) & !127,
            AluOp::Lzc => {
                // An LZC before the check may see a non-positive area; it
                // gives 0, and the check discards or restarts.
                let (m, e) = if a > 0 {
                    let e = 63 - a.leading_zeros();
                    (a << (33 - e), i64::from(e))
                } else {
                    (0, 0)
                };
                self.pending.push(Write {
                    at: now + 1,
                    target: Target::E,
                    value: e,
                });
                m
            }
            AluOp::Rcpld => {
                let index = usize::try_from((a >> 23) & 0x3FF).expect("10 bits");
                self.write(now + 2, 0, w.dest, SEED_TABLE[index]);
                return;
            }
            AluOp::Tst => {
                self.pending.push(Write {
                    at: now + 1,
                    target: Target::Flags,
                    value: i64::from(a < 0) << 1 | i64::from(a == 0),
                });
                return;
            }
            AluOp::Addv => r36(a + (b << 16)),
        };
        self.write(now + 1, 0, w.dest, value);
    }

    /// Issues a MUL word in cycle `now`.
    fn mul(&mut self, now: usize, w: MulWord) {
        if w.op == MulOp::Nop {
            return;
        }
        let a = self.read(w.a);
        let b = self.read(w.b);
        assert_eq!(wrap(b, 18), b, "18-bit operand out of range: {b}");
        self.products.push(Stage {
            at: now + MUL_SHIFT_STAGE - 1,
            word: w,
            value: a * b,
        });
    }

    /// The end of cycle `now`: shift the sums due, accumulate the terms
    /// due, land the writes due.
    fn end_of_cycle(&mut self, now: usize) {
        let e = u32::try_from(self.e).expect("E is 0 to 32");
        let due: Vec<Stage> = self.sums.iter().filter(|s| s.at == now).copied().collect();
        self.sums.retain(|s| s.at != now);
        for s in due {
            let w = s.word;
            let shift = u32::from(w.shift) + if w.by_e { e } else { 0 };
            let value = post_shift(s.value, shift, w.round, w.sat);
            self.write(now, 1, w.dest, value);
        }
        let due: Vec<Stage> = self
            .products
            .iter()
            .filter(|s| s.at == now)
            .copied()
            .collect();
        self.products.retain(|s| s.at != now);
        for p in due {
            self.acc = match p.word.op {
                MulOp::Mac => self.acc + p.value,
                MulOp::Msub => self.acc - p.value,
                MulOp::Mul | MulOp::Nop => p.value,
            };
            self.sums.push(Stage {
                at: now + 1,
                word: p.word,
                value: self.acc,
            });
        }
        let landing: Vec<Write> = self
            .pending
            .iter()
            .filter(|w| w.at == now)
            .copied()
            .collect();
        self.pending.retain(|w| w.at != now);
        let first = self.trace.len();
        for w in landing {
            let (port, addr) = match w.target {
                Target::Reg(bank, r) => {
                    self.banks[bank][usize::from(r)] = w.value;
                    (
                        if bank == 0 {
                            Port::AluBank
                        } else {
                            Port::MulBank
                        },
                        r,
                    )
                }
                Target::Out(o) => {
                    self.outputs[usize::from(o)] = Some(w.value);
                    (
                        if o < out::AB {
                            Port::MulOut
                        } else {
                            Port::AluOut
                        },
                        o,
                    )
                }
                Target::E => {
                    self.e = w.value;
                    continue;
                }
                Target::Flags => {
                    self.flags = w.value;
                    continue;
                }
            };
            // Each memory has one write port.
            assert!(
                self.trace[first..].iter().all(|t| t.port != port),
                "two writes to {port:?} in cycle {now}"
            );
            self.trace.push(Traced {
                cycle: now,
                port,
                addr,
                value: w.value,
            });
        }
    }

    /// Whether anything is still on its way to a register.
    fn busy(&self) -> bool {
        !(self.pending.is_empty() && self.products.is_empty() && self.sums.is_empty())
    }

    /// The outputs as a [`Setup`].
    fn result(&self, flipped: bool) -> Setup {
        let slot = |o: u8| self.outputs[usize::from(o)].expect("output written");
        let edge = |i: u8| Edge {
            a: Delta::from_count(slot(out::AB + 2 * i)),
            b: Delta::from_count(slot(out::AB + 2 * i + 1)),
            e: Wide::from_count(slot(out::E + i)),
        };
        let mut planes = [None; 8];
        for attr in Attr::ALL {
            let i = u8::try_from(attr.index()).expect("8 attributes");
            if self.outputs[usize::from(out::START + i)].is_some() {
                planes[attr.index()] = Some(Plane {
                    start: Iter::from_count(slot(out::START + i)),
                    dx: Iter::from_count(slot(out::DX + 2 * i)),
                    dy: Iter::from_count(slot(out::DX + 2 * i + 1)),
                });
            }
        }
        Setup {
            flipped,
            edges: [edge(0), edge(1), edge(2)],
            xs: Coord::from_count(slot(out::BOX)),
            ys: Coord::from_count(slot(out::BOX + 1)),
            xmax: Coord::from_count(slot(out::BOX + 2)),
            ymax: Coord::from_count(slot(out::BOX + 3)),
            e: u32::try_from(self.e).expect("E"),
            planes,
        }
    }
}

/// Runs a program on one triangle.
///
/// # Arguments
///
/// * `rom` - The program's micro-words, from its first.
/// * `tri` - The vertices, in the order the host sent them.
/// * `cull` - Which winding to discard.
///
/// # Returns
///
/// The cycles taken and the outputs, or why the triangle was discarded.
///
/// # Panics
///
/// If the program runs past its last word without an END, ends with
/// writes still in flight, or reads an 18-bit operand that doesn't fit.
pub fn run(rom: &Rom, tri: &Triangle, cull: CullMode) -> Run {
    let mut engine = Engine {
        tri,
        roles: [0, 1, 2],
        banks: [[0; BANK_SIZE as usize]; 2],
        outputs: [None; out::SLOTS],
        e: 0,
        flags: 0,
        acc: 0,
        pending: Vec::new(),
        products: Vec::new(),
        sums: Vec::new(),
        trace: Vec::new(),
    };
    let mut pc = 0;
    let mut flipped = false;
    let mut now = 0;
    loop {
        let mul = MulWord::decode(rom.mul[pc]);
        let alu = AluWord::decode(rom.alu[pc]);
        engine.mul(now, mul);
        engine.alu(now, alu);
        // The sequencer acts on this cycle's word, with the flags as they
        // are at its start.
        let seq = alu.seq;
        let (n, z) = (engine.flags & 2 != 0, engine.flags & 1 != 0);
        engine.end_of_cycle(now);
        now += 1;
        match seq {
            Seq::Next => pc += 1,
            Seq::Check => {
                // Culling looks at the winding as sent: once flipped, the
                // area is positive and the triangle was already kept.
                let culled = !flipped
                    && match cull {
                        CullMode::None => false,
                        CullMode::Back => n,
                        CullMode::Front => !n,
                    };
                if z {
                    return Run {
                        cycles: now,
                        result: Err(Reject::ZeroArea),
                        trace: engine.trace,
                    };
                }
                if culled {
                    return Run {
                        cycles: now,
                        result: Err(Reject::Culled),
                        trace: engine.trace,
                    };
                }
                if n {
                    // Swap roles 1 and 2 and start again. Writes still in
                    // flight land on registers the restarted program
                    // writes again before it reads them.
                    engine.roles.swap(1, 2);
                    flipped = true;
                    pc = 0;
                } else {
                    pc += 1;
                }
            }
            Seq::End => {
                assert!(!engine.busy(), "END with writes in flight");
                return Run {
                    cycles: now,
                    result: Ok(engine.result(flipped)),
                    trace: engine.trace.clone(),
                };
            }
        }
        assert!(pc < rom.mul.len(), "ran past the program");
    }
}
