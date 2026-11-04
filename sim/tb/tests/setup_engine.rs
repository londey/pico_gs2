//! The setup engine (`hw/setup.snt`, through `hw/setup_tb.snt`) against
//! its golden model, the microcode interpreter in `models/setup`: for
//! random triangles, both programs and every cull mode, the same writes to
//! the same banks and output slots in the same cycles, the same result,
//! and the committed output buffer read back slot by slot.
//!
//! The engine's inputs on every cycle are recorded as
//! `build/stimulus/setup_tb.random.csv`, which `just check` replays with
//! `--cosim` against the Verilog.

use std::error::Error;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use setup_model::emit::{PROGRAMS, PROGRAM_WORDS};
use setup_model::interp::{run, Port, Rom, Traced};
use setup_model::{schedule, setup_program, Attr, Coord, CullMode, Reject, Triangle, Vertex};
use setup_tb_sim::{Inputs, Outputs, SetupTb};

/// Triangles in the test.
const TRIANGLES: usize = 1_500;

/// Words in a 36-bit register.
const MASK36: u64 = (1 << 36) - 1;

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

/// A random triangle: tiny to huge, anywhere in the guard band, sometimes
/// with coincident vertices.
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
    if rng.next().is_multiple_of(40) {
        p[2] = p[1];
    }
    p.map(|(x, y)| {
        let w = rng.next().to_le_bytes();
        let h = rng.next().to_le_bytes();
        let k = rng.next().to_le_bytes();
        Vertex {
            x: Coord::from_count(x),
            y: Coord::from_count(y),
            r: w[0],
            g: w[1],
            b: w[2],
            a: w[3],
            z: u16::from_le_bytes([h[0], h[1]]),
            s: i16::from_le_bytes([h[2], h[3]]),
            t: i16::from_le_bytes([k[0], k[1]]),
            q: u16::from_le_bytes([k[2], k[3]]),
        }
    })
}

/// A vertex's ten fields as the ring holds them: 36-bit words, signed
/// fields sign-extended.
fn fields(v: &Vertex) -> [u64; 10] {
    let word = |x: i64| u64::from_ne_bytes(x.to_ne_bytes()) & MASK36;
    let mut f = [0; 10];
    f[0] = word(v.x.to_count());
    f[1] = word(v.y.to_count());
    for attr in Attr::ALL {
        f[2 + attr.index()] = word(attr.of(v));
    }
    f
}

/// The engine, stepped one cycle at a time, with its inputs recorded.
struct Bench<W: Write> {
    /// The simulator.
    sim: SetupTb,
    /// Where the stimulus goes.
    stimulus: W,
}

impl<W: Write> Bench<W> {
    /// One cycle: records the inputs, steps, returns the outputs.
    fn step(&mut self, i: &Inputs) -> std::io::Result<Outputs> {
        writeln!(
            self.stimulus,
            "{},{},{},{},{},{},{},{},{}",
            i.load_is_some,
            i.load_payload_slot,
            i.load_payload_field,
            i.load_payload_value,
            i.start_is_some,
            i.start_payload_roles,
            i.start_payload_cull,
            i.start_payload_untextured,
            i.out_raddr
        )?;
        Ok(self.sim.step(i))
    }
}

/// An idle cycle's inputs.
fn idle() -> Inputs {
    Inputs {
        load_is_some: 0,
        load_payload_slot: 0,
        load_payload_field: 0,
        load_payload_value: 0,
        start_is_some: 0,
        start_payload_roles: 0,
        start_payload_cull: 0,
        start_payload_untextured: 0,
        out_raddr: 0,
    }
}

/// The writes the engine drives this cycle, as the interpreter traces them
/// (output slots by their low 5 bits: the engine adds the buffer bank).
fn writes(o: &Outputs, cycle: usize) -> Vec<Traced> {
    let mut w = Vec::new();
    let mut add = |we: u8, port: Port, addr: u8, data: u64| {
        if we == 1 {
            w.push(Traced {
                cycle,
                port,
                addr: addr & 0x1F,
                value: i64::try_from(data).expect("36 bits"),
            });
        }
    };
    add(
        o.out_debug_alu_bank_we,
        Port::AluBank,
        o.out_debug_alu_bank_addr,
        o.out_debug_alu_bank_data,
    );
    add(
        o.out_debug_mul_bank_we,
        Port::MulBank,
        o.out_debug_mul_bank_addr,
        o.out_debug_mul_bank_data,
    );
    add(
        o.out_debug_alu_out_we,
        Port::AluOut,
        o.out_debug_alu_out_addr,
        o.out_debug_alu_out_data,
    );
    add(
        o.out_debug_mul_out_we,
        Port::MulOut,
        o.out_debug_mul_out_addr,
        o.out_debug_mul_out_data,
    );
    w
}

/// A trace entry with its value as a 36-bit word and its slot's low bits,
/// to compare with the engine's.
fn normalised(t: &Traced) -> Traced {
    Traced {
        addr: t.addr & 0x1F,
        value: i64::try_from(u64::from_ne_bytes(t.value.to_ne_bytes()) & MASK36).expect("36 bits"),
        ..*t
    }
}

/// The output slots a program writes, with their words.
fn output_slots(trace: &[Traced]) -> Vec<(u8, u64)> {
    let mut slots: Vec<(u8, u64)> = Vec::new();
    for t in trace
        .iter()
        .filter(|t| matches!(t.port, Port::AluOut | Port::MulOut))
    {
        let word = u64::from_ne_bytes(t.value.to_ne_bytes()) & MASK36;
        match slots.iter_mut().find(|(s, _)| *s == t.addr) {
            Some(slot) => slot.1 = word,
            None => slots.push((t.addr, word)),
        }
    }
    slots
}

/// Runs one triangle through the engine and checks it against the
/// interpreter.
fn check_triangle<W: Write>(
    bench: &mut Bench<W>,
    rom: &Rom,
    untextured: bool,
    tri: &Triangle,
    cull: CullMode,
) -> std::io::Result<()> {
    // Load the vertices into slots 0 to 2.
    for (slot, v) in tri.iter().enumerate() {
        for (field, value) in fields(v).into_iter().enumerate() {
            bench.step(&Inputs {
                load_is_some: 1,
                load_payload_slot: u8::try_from(slot).expect("small"),
                load_payload_field: u8::try_from(field).expect("small"),
                load_payload_value: value,
                ..idle()
            })?;
        }
    }
    let expected = run(rom, tri, cull);
    let cull_code = match cull {
        CullMode::None => 0,
        CullMode::Back => 1,
        CullMode::Front => 2,
    };
    bench.step(&Inputs {
        start_is_some: 1,
        start_payload_roles: 0b00_01_10,
        start_payload_cull: cull_code,
        start_payload_untextured: u8::from(untextured),
        ..idle()
    })?;
    // Cycle 0 is the first issue. The engine reports done the cycle after
    // its END or abort.
    let mut got = Vec::new();
    let mut cycle = 0;
    let done = loop {
        let o = bench.step(&idle())?;
        if cycle < expected.cycles {
            got.extend(writes(&o, cycle));
        }
        if o.out_done == 1 {
            break o;
        }
        cycle += 1;
        assert!(cycle < 4 * PROGRAM_WORDS, "the engine never finished");
    };
    assert_eq!(cycle, expected.cycles, "cycles for {tri:?} {cull:?}");
    // Writes in the same cycle go to different ports: compare in one order.
    let key = |t: &Traced| (t.cycle, t.port as u8);
    got.sort_by_key(key);
    let mut want: Vec<Traced> = expected.trace.iter().map(normalised).collect();
    want.sort_by_key(key);
    assert_eq!(got, want, "writes for {tri:?} {cull:?}");
    let (result, flipped) = match &expected.result {
        Ok(s) => (0, u8::from(s.flipped)),
        Err(Reject::ZeroArea) => (1, 0),
        Err(Reject::Culled) => (2, 0),
    };
    assert_eq!(
        (done.out_result, done.out_debug_flipped),
        (result, flipped),
        "{tri:?} {cull:?}"
    );

    // A kept triangle's outputs are committed: read every slot back. A
    // read's word comes out the cycle after its address goes in.
    if expected.result.is_ok() {
        let mut previous: Option<(u8, u64)> = None;
        for &(slot, word) in &output_slots(&expected.trace) {
            let o = bench.step(&Inputs {
                out_raddr: slot,
                ..idle()
            })?;
            if let Some((s, w)) = previous {
                assert_eq!(o.out_rdata, w, "slot {s:#x} read back");
            }
            previous = Some((slot, word));
        }
        let o = bench.step(&idle())?;
        if let Some((s, w)) = previous {
            assert_eq!(o.out_rdata, w, "slot {s:#x} read back");
        }
    }
    Ok(())
}

/// The engine matches the interpreter, cycle for cycle.
#[test]
fn engine_matches_the_interpreter() -> Result<(), Box<dyn Error>> {
    // The ROMs the engine was built with are setup-gen's, from the same
    // generator: the programs at their bases.
    let roms: Vec<(bool, Rom)> = PROGRAMS
        .iter()
        .enumerate()
        .map(|(i, &(_, attrs))| (i == 1, schedule(&setup_program(attrs)).rom()))
        .collect();
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "..", "build", "stimulus"]
        .iter()
        .collect();
    fs::create_dir_all(&path)?;
    let mut stimulus = BufWriter::new(File::create(path.join("setup_tb.random.csv"))?);
    writeln!(
        stimulus,
        "# Recorded by sim/tb/tests/setup_engine.rs; replayed by `just check`."
    )?;
    writeln!(
        stimulus,
        "load_is_some,load_payload_slot,load_payload_field,load_payload_value,\
         start_is_some,start_payload_roles,start_payload_cull,start_payload_untextured,out_raddr"
    )?;
    let mut bench = Bench {
        sim: SetupTb::new(),
        stimulus,
    };
    let mut rng = Lcg(5);
    for n in 0..TRIANGLES {
        let (untextured, rom) = &roms[n % 2];
        let cull = [CullMode::None, CullMode::Back, CullMode::Front][(n / 2) % 3];
        let tri = triangle(&mut rng);
        check_triangle(&mut bench, rom, *untextured, &tri, cull)?;
    }
    bench.stimulus.flush()?;
    Ok(())
}
