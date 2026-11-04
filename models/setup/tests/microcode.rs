//! The scheduled microcode, run by the cycle-level interpreter, against the
//! bit-exact model (brief §8): the same outputs bit for bit, or the same
//! rejection, for every triangle of a random corpus; cycle counts fixed by
//! the schedule; and the schedule's report within its limits.

use setup_model::interp::{run, Rom};
use setup_model::isa::{AluWord, MulWord};
use setup_model::{
    schedule, setup, setup_program, AttrSet, Coord, CullMode, Schedule, Triangle, Vertex,
};

/// Random triangles per program and cull mode.
const TRIANGLES: usize = 3_000;

/// Longest the full program may be, in cycles: what it schedules to now, as
/// a regression limit (ADR 0004). The brief's target is 85.
const FULL_MAX: usize = 94;

/// Longest the untextured program may be, likewise.
const UNTEXTURED_MAX: usize = 74;

/// A small linear congruential generator, so the test is repeatable.
struct Lcg(u64);

impl Lcg {
    /// The next 32 pseudo-random bits.
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        u32::try_from(self.0 >> 32).expect("32 bits")
    }

    /// A pseudo-random integer in `lo..hi`.
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + i64::from(self.next()) % (hi - lo)
    }
}

/// A random triangle: sometimes tiny, sometimes huge, sometimes with
/// coincident vertices, anywhere in the guard band.
fn triangle(rng: &mut Lcg) -> Triangle {
    let size = [1, 8, 64, 400, 2048][usize::try_from(rng.next() % 5).expect("small")];
    let cx = rng.range(-64 * 16, 704 * 16);
    let cy = rng.range(-64 * 16, 424 * 16);
    let point = |rng: &mut Lcg| {
        (
            (cx + rng.range(-size * 16, size * 16 + 1)).clamp(-2048 * 16, 2048 * 16 - 1),
            (cy + rng.range(-size * 16, size * 16 + 1)).clamp(-2048 * 16, 2048 * 16 - 1),
        )
    };
    let mut p = [point(rng), point(rng), point(rng)];
    if rng.next().is_multiple_of(50) {
        p[2] = p[1];
    }
    p.map(|(x, y)| {
        let w = rng.next();
        let h = rng.next();
        Vertex {
            x: Coord::from_count(x),
            y: Coord::from_count(y),
            r: w.to_le_bytes()[0],
            g: w.to_le_bytes()[1],
            b: w.to_le_bytes()[2],
            a: w.to_le_bytes()[3],
            z: u16::try_from(h & 0xFFFF).expect("16 bits"),
            s: i16::from_le_bytes((h >> 16).to_le_bytes()[..2].try_into().expect("2 bytes")),
            t: i16::from_le_bytes(rng.next().to_le_bytes()[..2].try_into().expect("2 bytes")),
            q: u16::try_from(rng.next() & 0xFFFF).expect("16 bits"),
        }
    })
}

/// Schedules a program and prints its report.
fn scheduled(name: &str, attrs: AttrSet) -> (Schedule, Rom) {
    let s = schedule(&setup_program(attrs));
    println!(
        "{name}: {} cycles (critical path {}), check at {}, MUL {} issues ({:.0}%), ALU {} ({:.0}%), temps ALU {} MUL {}",
        s.cycles(),
        s.critical_path,
        s.check,
        s.mul_issues,
        100.0 * f64::from(u32::try_from(s.mul_issues).expect("small"))
            / f64::from(u32::try_from(s.cycles()).expect("small")),
        s.alu_issues,
        100.0 * f64::from(u32::try_from(s.alu_issues).expect("small"))
            / f64::from(u32::try_from(s.cycles()).expect("small")),
        s.peak_temps.0,
        s.peak_temps.1,
    );
    let rom = s.rom();
    (s, rom)
}

/// The interpreter gives exactly the bit-exact model's results, in the
/// cycles the schedule says.
fn matches_the_model(name: &str, attrs: AttrSet, max_cycles: usize) {
    let (s, rom) = scheduled(name, attrs);
    assert!(s.cycles() <= max_cycles, "{name}: {} cycles", s.cycles());
    let mut rng = Lcg(17);
    for cull in [CullMode::None, CullMode::Back, CullMode::Front] {
        for _ in 0..TRIANGLES {
            let tri = triangle(&mut rng);
            let expected = setup(&tri, cull, attrs);
            let got = run(&rom, &tri, cull);
            assert_eq!(got.result, expected, "{name} {cull:?} {tri:?}");
            let cycles = match expected {
                Err(_) => s.check + 1,
                Ok(r) if r.flipped => s.check + 1 + s.cycles(),
                Ok(_) => s.cycles(),
            };
            assert_eq!(got.cycles, cycles, "{name} {cull:?} cycles");
        }
    }
}

/// The full program.
#[test]
fn full_program_matches_the_model() {
    matches_the_model("full", AttrSet::FULL, FULL_MAX);
}

/// The untextured program.
#[test]
fn untextured_program_matches_the_model() {
    matches_the_model("untextured", AttrSet::UNTEXTURED, UNTEXTURED_MAX);
}

/// Every word decodes to what was encoded.
#[test]
fn words_round_trip() {
    for attrs in [AttrSet::FULL, AttrSet::UNTEXTURED] {
        let s = schedule(&setup_program(attrs));
        for w in &s.words {
            assert_eq!(MulWord::decode(w.mul.encode()), w.mul);
            assert_eq!(AluWord::decode(w.alu.encode()), w.alu);
            assert!(w.mul.encode() < 1 << MulWord::BITS);
            assert!(w.alu.encode() < 1 << AluWord::BITS);
        }
    }
}
