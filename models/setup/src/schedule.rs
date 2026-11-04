//! The scheduler and register allocator (brief §7): from a [`Program`] to
//! micro-words.
//!
//! List scheduling, one MUL-slot and one ALU-slot issue per cycle, highest
//! critical path first. A multiplier sum issues its terms on consecutive
//! cycles. A value can be read from the cycle its producer's latency
//! allows (`isa::*_LATENCY`). After scheduling, values that something
//! reads get a temporary in their producer's bank; a register is reused
//! once its last read has issued (memories read first, so a write in the
//! same cycle as the last read is safe).

use std::collections::HashMap;

use crate::interp::Rom;
use crate::isa::{
    read, AluOp, AluWord, Dest, MicroWord, MulWord, Seq, ALU_LATENCY, BANK_SIZE, MUL_LATENCY,
    MUL_SHIFT_STAGE, RCPLD_LATENCY, TST_LATENCY,
};
use crate::program::{Inst, Kind, Program, Src, ValueId};

/// A scheduled program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    /// The micro-words, one per cycle; the last has [`Seq::End`].
    pub words: Vec<MicroWord>,

    /// The cycle whose word has [`Seq::Check`].
    pub check: usize,

    /// Each instruction's first issue cycle.
    pub issue: Vec<usize>,

    /// Multiplier-slot issues.
    pub mul_issues: usize,

    /// ALU-slot issues.
    pub alu_issues: usize,

    /// The longest dependency chain, in cycles: the shortest the program
    /// could be with unlimited slots.
    pub critical_path: usize,

    /// Most temporaries live at once, in the ALU bank and the MUL bank.
    pub peak_temps: (usize, usize),
}

impl Schedule {
    /// Cycles from the first word to the end, inclusive.
    pub fn cycles(&self) -> usize {
        self.words.len()
    }

    /// The program as ROM words.
    ///
    /// # Returns
    ///
    /// Each cycle's encoded MUL and ALU words.
    pub fn rom(&self) -> Rom {
        Rom {
            mul: self.words.iter().map(|w| w.mul.encode()).collect(),
            alu: self.words.iter().map(|w| w.alu.encode()).collect(),
        }
    }
}

/// Whether an instruction is on the multiplier.
fn is_mul(inst: &Inst) -> bool {
    matches!(inst.kind, Kind::Mul { .. })
}

/// Issues an instruction occupies: one per term on the multiplier.
fn width(inst: &Inst) -> usize {
    match &inst.kind {
        Kind::Mul { terms, .. } => terms.len(),
        Kind::Alu { .. } => 1,
    }
}

/// Cycles from an instruction's first issue to the first issue that can
/// read its value.
fn latency(inst: &Inst) -> usize {
    match &inst.kind {
        Kind::Mul { terms, .. } => terms.len() - 1 + MUL_LATENCY,
        Kind::Alu {
            op: AluOp::Rcpld, ..
        } => RCPLD_LATENCY,
        Kind::Alu { .. } => ALU_LATENCY,
    }
}

/// The cycle, relative to its first issue, in which an instruction writes
/// its register.
fn write_offset(inst: &Inst) -> usize {
    latency(inst) - 1
}

/// The values an instruction reads, each with the issue offset (from its
/// first issue) at which it is read.
fn reads(inst: &Inst) -> Vec<(ValueId, usize)> {
    match &inst.kind {
        Kind::Alu { a, b, .. } => [a, b]
            .iter()
            .filter_map(|s| match s {
                Src::Value(v) => Some((*v, 0)),
                _ => None,
            })
            .collect(),
        Kind::Mul { terms, .. } => terms
            .iter()
            .enumerate()
            .flat_map(|(t, term)| [(term.a, t), (term.b, t)])
            .collect(),
    }
}

/// Schedules a program.
///
/// # Arguments
///
/// * `program` - The program.
///
/// # Returns
///
/// Its micro-words and a report.
///
/// # Panics
///
/// If a bank runs out of registers.
pub fn schedule(program: &Program) -> Schedule {
    let insts = &program.insts;
    let n = insts.len();

    // Height: the longest chain from an instruction's first issue to the
    // end of the program, which is the priority.
    let mut height = vec![0; n];
    for i in (0..n).rev() {
        height[i] = latency(&insts[i]);
    }
    for i in (0..n).rev() {
        for (v, offset) in reads(&insts[i]) {
            let through = latency(&insts[v]) - offset + height[i];
            height[v] = height[v].max(through);
        }
    }
    // The area's check is the most urgent: put its TST first among equals.
    height[program.check_after] += n;

    let mut issue: Vec<Option<usize>> = vec![None; n];
    let mut mul_busy: Vec<bool> = Vec::new();
    let mut alu_busy: Vec<bool> = Vec::new();
    let busy = |slots: &Vec<bool>, c: usize| slots.get(c).copied().unwrap_or(false);
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(height[i]), i));

    let mut cycle = 0;
    while issue.iter().any(Option::is_none) {
        for &i in &order {
            if issue[i].is_some() {
                continue;
            }
            let inst = &insts[i];
            // Every value read must be available when it is read.
            let ready = reads(inst).iter().all(|&(v, offset)| {
                issue[v].is_some_and(|iv| iv + latency(&insts[v]) <= cycle + offset)
            });
            if !ready {
                continue;
            }
            let slots = if is_mul(inst) {
                &mut mul_busy
            } else {
                &mut alu_busy
            };
            if (0..width(inst)).any(|t| busy(slots, cycle + t)) {
                continue;
            }
            for t in 0..width(inst) {
                if slots.len() <= cycle + t {
                    slots.resize(cycle + t + 1, false);
                }
                slots[cycle + t] = true;
            }
            issue[i] = Some(cycle);
        }
        cycle += 1;
    }
    let issue: Vec<usize> = issue.into_iter().map(|c| c.expect("scheduled")).collect();

    // The check, and the end: after every write.
    let check = issue[program.check_after] + TST_LATENCY;
    let last_write = (0..n)
        .map(|i| issue[i] + write_offset(&insts[i]))
        .max()
        .unwrap_or(0);
    let end = last_write.max(check + 1);

    let (temps, peak_temps) = allocate(program, &issue);
    let mut words = encode(program, &issue, &temps, end + 1);
    words[check].alu.seq = Seq::Check;
    words[end].alu.seq = Seq::End;

    // E is written by the LZC; every sum that shifts by it must reach its
    // shift stage after that write.
    let lzc = (0..n).find(|&i| matches!(insts[i].kind, Kind::Alu { op: AluOp::Lzc, .. }));
    if let Some(lzc) = lzc {
        for (i, inst) in insts.iter().enumerate() {
            if let Kind::Mul { terms, post } = &inst.kind {
                if post.by_e {
                    let stage = issue[i] + terms.len() - 1 + MUL_SHIFT_STAGE;
                    assert!(
                        stage > issue[lzc] + 1,
                        "{} shifts by E before it is written",
                        inst.name
                    );
                }
            }
        }
    }

    Schedule {
        words,
        check,
        mul_issues: insts.iter().filter(|i| is_mul(i)).map(width).sum(),
        alu_issues: insts.iter().filter(|i| !is_mul(i)).count(),
        critical_path: height
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != program.check_after)
            .map(|(_, h)| *h)
            .max()
            .unwrap_or(0),
        issue,
        peak_temps,
    }
}

/// Builds the micro-words from a schedule and register allocation.
///
/// # Arguments
///
/// * `program` - The program.
/// * `issue` - Each instruction's first issue cycle.
/// * `temps` - Each value's register, if it has one.
/// * `len` - Words in the program.
///
/// # Returns
///
/// The words, with every sequencer field [`Seq::Next`].
fn encode(program: &Program, issue: &[usize], temps: &[Option<u8>], len: usize) -> Vec<MicroWord> {
    let insts = &program.insts;
    let mut words = vec![MicroWord::default(); len];
    for (i, inst) in insts.iter().enumerate() {
        let dest = Dest {
            temp: temps[i],
            out: inst.out,
        };
        let addr = |s: &Src| -> u8 {
            match s {
                Src::Vertex(role, f) => read::VERTEX + (role << 4 | f),
                Src::Const(c) => c.addr(),
                Src::Value(v) => {
                    let bank = if is_mul(&insts[*v]) {
                        read::MUL_BANK
                    } else {
                        read::ALU_BANK
                    };
                    bank + temps[*v].expect("a value that is read has a register")
                }
            }
        };
        match &inst.kind {
            Kind::Alu { op, a, b } => {
                let w = &mut words[issue[i]].alu;
                *w = AluWord {
                    op: *op,
                    a: addr(a),
                    b: addr(b),
                    dest,
                    seq: w.seq,
                };
            }
            Kind::Mul { terms, post } => {
                for (t, term) in terms.iter().enumerate() {
                    let last = t + 1 == terms.len();
                    words[issue[i] + t].mul = MulWord {
                        op: term.op,
                        a: addr(&Src::Value(term.a)),
                        b: addr(&Src::Value(term.b)),
                        dest: if last { dest } else { Dest::default() },
                        shift: post.shift,
                        by_e: post.by_e,
                        round: post.round,
                        sat: post.sat,
                    };
                }
            }
        }
    }
    words
}

/// Allocates temporaries.
///
/// # Arguments
///
/// * `program` - The program.
/// * `issue` - Each instruction's first issue cycle.
///
/// # Returns
///
/// Each value's register in its producer's bank (`None` if nothing reads
/// it), and the peak registers in use in the ALU and MUL banks.
///
/// # Panics
///
/// If more than [`BANK_SIZE`] registers are live at once in a bank.
fn allocate(program: &Program, issue: &[usize]) -> (Vec<Option<u8>>, (usize, usize)) {
    let insts = &program.insts;
    let n = insts.len();
    // Each value's last read.
    let mut last_read: HashMap<ValueId, usize> = HashMap::new();
    for (i, inst) in insts.iter().enumerate() {
        for (v, offset) in reads(inst) {
            let at = issue[i] + offset;
            let e = last_read.entry(v).or_insert(at);
            *e = (*e).max(at);
        }
    }
    let mut temps = vec![None; n];
    let mut peaks = [0; 2];
    for (bank, peak) in peaks.iter_mut().enumerate() {
        let mut values: Vec<ValueId> = (0..n)
            .filter(|&v| usize::from(is_mul(&insts[v])) == bank && last_read.contains_key(&v))
            .collect();
        values.sort_by_key(|&v| issue[v] + write_offset(&insts[v]));
        // Each register's last read so far.
        let mut free_at: Vec<usize> = Vec::new();
        for v in values {
            let write = issue[v] + write_offset(&insts[v]);
            let reg = if let Some(r) = free_at.iter().position(|&f| f <= write) {
                r
            } else {
                free_at.push(0);
                free_at.len() - 1
            };
            assert!(
                reg < usize::from(BANK_SIZE),
                "bank {bank} out of registers at {}",
                insts[v].name
            );
            free_at[reg] = last_read[&v];
            temps[v] = Some(u8::try_from(reg).expect("under 32"));
            *peak = (*peak).max(free_at.len());
        }
    }
    (temps, (peaks[0], peaks[1]))
}
