//! The generator's outputs: the micro-ROMs and the seed table as hex for
//! Sinter's `include_hex!`, and the schedule report.

use std::fmt::Write;

use crate::isa::{AluWord, MulWord};
use crate::program::setup_program;
use crate::reciprocal::SEED_TABLE;
use crate::schedule::{schedule, Schedule};
use crate::triangle::AttrSet;

/// Words in the micro-ROMs.
pub const ROM_WORDS: usize = 256;

/// Words each program may take: program `i` starts at word
/// `i * PROGRAM_WORDS`, so the engine selects one with a constant.
pub const PROGRAM_WORDS: usize = 128;

/// The programs in ROM order, with their names.
pub const PROGRAMS: [(&str, AttrSet); 2] =
    [("full", AttrSet::FULL), ("untextured", AttrSet::UNTEXTURED)];

/// Everything the generator writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generated {
    /// The MUL-slot ROM, one word per line.
    pub mul_hex: String,

    /// The ALU-slot ROM.
    pub alu_hex: String,

    /// The seed table.
    pub seed_hex: String,

    /// The built-in scene, the spinning cube, as transactions (ADR 0008):
    /// each one's address byte (bit 7 set for a read; 0xFF the end).
    pub scene_addr_hex: String,

    /// Each transaction's data.
    pub scene_data_hex: String,

    /// The schedule report.
    pub report: String,

    /// Each program's first word in the ROMs.
    pub bases: Vec<(&'static str, usize)>,
}

/// One hex line per word, `digits` wide, after a comment header.
fn hex(header: &str, words: impl Iterator<Item = u64>, digits: usize) -> String {
    let mut s = format!("// {header}\n");
    for w in words {
        let _ = writeln!(s, "{w:0digits$x}");
    }
    s
}

/// Schedules every program and renders the generator's outputs.
///
/// # Returns
///
/// The ROM and seed contents and the report.
pub fn generate() -> Generated {
    let schedules: Vec<(&str, Schedule)> = PROGRAMS
        .iter()
        .map(|&(name, attrs)| (name, schedule(&setup_program(attrs))))
        .collect();
    let mut bases = Vec::new();
    let mut mul = Vec::new();
    let mut alu = Vec::new();
    let mut report =
        String::from("program      cycles  critical  check  mul  alu  temps(alu,mul)  base\n");
    for (i, (name, s)) in schedules.iter().enumerate() {
        assert!(
            s.cycles() <= PROGRAM_WORDS,
            "{name} doesn't fit in {PROGRAM_WORDS} words"
        );
        // Unused words are NOPs (all zero).
        mul.resize(i * PROGRAM_WORDS, 0);
        alu.resize(i * PROGRAM_WORDS, 0);
        bases.push((*name, mul.len()));
        let _ = writeln!(
            report,
            "{name:<12} {:>6} {:>9} {:>6} {:>4} {:>4} {:>8},{:<6} {:>4}",
            s.cycles(),
            s.critical_path,
            s.check,
            s.mul_issues,
            s.alu_issues,
            s.peak_temps.0,
            s.peak_temps.1,
            mul.len()
        );
        let rom = s.rom();
        mul.extend(rom.mul);
        alu.extend(rom.alu);
    }
    mul.resize(ROM_WORDS, 0);
    alu.resize(ROM_WORDS, 0);
    let scene = crate::scene::rom(&crate::scene::cube_scene());
    Generated {
        mul_hex: hex(
            "setup engine MUL-slot micro-ROM (generated)",
            mul.into_iter(),
            10,
        ),
        alu_hex: hex(
            "setup engine ALU-slot micro-ROM (generated)",
            alu.into_iter(),
            9,
        ),
        seed_hex: hex(
            "reciprocal seeds, Q1.17 (generated)",
            SEED_TABLE
                .iter()
                .map(|&s| u64::try_from(s).expect("positive")),
            5,
        ),
        scene_addr_hex: hex(
            "the built-in scene, the spinning cube: transaction addresses (generated)",
            scene.iter().map(|&(a, _)| u64::from(a)),
            2,
        ),
        scene_data_hex: hex(
            "the built-in scene, the spinning cube: transaction data (generated)",
            scene.iter().map(|&(_, d)| d),
            16,
        ),
        report,
        bases,
    }
}

/// The hex digit counts above (10 and 9) hold the words.
const _: () = assert!(MulWord::BITS <= 40 && AluWord::BITS <= 36);
