//! Test of the SDRAM controller (`hw/sdram.snt`) on its own, against the
//! W9825G6KH model: pseudo-random write and read runs across all banks, with
//! every word read back checked and the model reporting no datasheet
//! violation.

use std::collections::{HashMap, VecDeque};

use sdram_controller_sim::{sdram__SdramController as SdramController, Inputs, Outputs};
use sdram_model::{Pins, COLUMNS};
use tb::SdramBus;

/// Runs in the test.
const RUNS: usize = 400;

/// Longest run the controller takes, in words.
const MAX_RUN: u32 = 32;

/// Cycle limit, well beyond what the runs need.
const MAX_CYCLES: u64 = 200_000;

/// A run of words to write or read.
#[derive(Clone, Copy, Debug)]
struct Run {
    /// Write (`true`) or read.
    write: bool,

    /// Word address of the first word.
    addr: u32,

    /// Words in the run, 1 to 32.
    len: u32,
}

/// A small linear congruential generator, so the test is repeatable.
struct Lcg(u64);

impl Lcg {
    /// The next 32 pseudo-random bits.
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 32) as u32
    }
}

/// The SDRAM pins the controller drives.
fn pins(o: &Outputs) -> Pins {
    Pins {
        cke: o.out_pins_cke == 1,
        cs_n: o.out_pins_cs_n == 1,
        ras_n: o.out_pins_ras_n == 1,
        cas_n: o.out_pins_cas_n == 1,
        we_n: o.out_pins_we_n == 1,
        ba: o.out_pins_ba,
        a: o.out_pins_a,
        dqm: o.out_pins_dqm,
        dq: (o.out_pins_dq_oe == 1).then_some(o.out_pins_dq_out),
    }
}

/// The data written to the `i`th word of all writes.
fn data(i: u32) -> u16 {
    (i.wrapping_mul(40_503) >> 3) as u16
}

/// Write runs over a spread of addresses, each followed (not always at
/// once) by a read of the same words, plus reads of words not yet written.
fn runs() -> Vec<Run> {
    let mut rng = Lcg(1);
    let mut runs = Vec::with_capacity(RUNS);
    let mut written: Vec<Run> = Vec::new();
    while runs.len() < RUNS {
        let len = rng.next() % MAX_RUN + 1;
        // A start column that keeps the run in one row; every so often, a
        // run that ends exactly at the row's last column.
        let column = if rng.next().is_multiple_of(8) {
            COLUMNS as u32 - len
        } else {
            rng.next() % (COLUMNS as u32 - len + 1)
        };
        let row = rng.next() % 8192;
        let bank = rng.next() % 4;
        let addr = (bank << 22) | (row << 9) | column;
        match rng.next() % 3 {
            0 if !written.is_empty() => {
                let pick = written.swap_remove(rng.next() as usize % written.len());
                runs.push(Run {
                    write: false,
                    ..pick
                });
            }
            1 => runs.push(Run {
                write: false,
                addr,
                len,
            }),
            _ => {
                let run = Run {
                    write: true,
                    addr,
                    len,
                };
                written.push(run);
                runs.push(run);
            }
        }
    }
    runs.extend(written.into_iter().map(|w| Run { write: false, ..w }));
    runs
}

/// Every word read comes back as last written (or zero), and the
/// controller never breaks the datasheet's rules.
#[test]
fn runs_read_back_and_meet_the_datasheet() {
    let mut sim = SdramController::new();
    let mut bus = SdramBus::new();
    let mut pending: VecDeque<Run> = runs().into();
    // Words of accepted write runs, not yet taken: (address, data).
    let mut to_write: VecDeque<(u32, u16)> = VecDeque::new();
    // Words of accepted read runs, not yet returned: (address, expected).
    let mut to_read: VecDeque<(u32, u16)> = VecDeque::new();
    // What the SDRAM should hold.
    let mut shadow: HashMap<u32, u16> = HashMap::new();
    let mut words = 0;
    let mut checked = 0;

    while (!pending.is_empty() || !to_write.is_empty() || !to_read.is_empty())
        && sim.time() < MAX_CYCLES
    {
        let req = pending.front().copied();
        let inputs = Inputs {
            req_is_some: u8::from(req.is_some()),
            req_payload_write: u8::from(req.is_some_and(|r| r.write)),
            req_payload_addr: req.map_or(0, |r| r.addr),
            req_payload_len: req.map_or(0, |r| r.len as u8),
            wdata: to_write.front().map_or(0, |&(_, d)| d),
            dq_in: bus.dq_in(),
        };
        let out = sim.step(&inputs);
        bus.clock(&pins(&out));

        if out.out_accepted == 1 {
            let run = pending.pop_front().expect("accepted without a request");
            for addr in run.addr..run.addr + run.len {
                if run.write {
                    to_write.push_back((addr, data(words)));
                    words += 1;
                } else {
                    // Runs are done in order, so a read sees every write
                    // accepted before it.
                    let pending_write = to_write
                        .iter()
                        .rev()
                        .find(|(a, _)| *a == addr)
                        .map(|&(_, d)| d);
                    let expected = pending_write
                        .or_else(|| shadow.get(&addr).copied())
                        .unwrap_or(0);
                    to_read.push_back((addr, expected));
                }
            }
        }
        if out.out_wtake == 1 {
            let (addr, d) = to_write.pop_front().expect("wtake with nothing to write");
            shadow.insert(addr, d);
        }
        if out.out_rdata_is_some == 1 {
            let (addr, expected) = to_read.pop_front().expect("read data nobody asked for");
            assert_eq!(
                out.out_rdata_payload,
                expected,
                "word {addr:#08x} at cycle {}",
                sim.time()
            );
            checked += 1;
        }
    }

    assert!(
        pending.is_empty() && to_write.is_empty() && to_read.is_empty(),
        "runs not finished in {MAX_CYCLES} cycles"
    );
    assert!(
        bus.sdram.violations().is_empty(),
        "{} violations: {:#?}",
        bus.sdram.violation_count(),
        bus.sdram.violations()
    );
    for (&addr, &d) in &shadow {
        let (bank, row, column) = (
            addr as usize >> 22,
            (addr as usize >> 9) & 0x1FFF,
            addr as usize & 0x1FF,
        );
        assert_eq!(
            bus.sdram.peek(bank, row, column),
            d,
            "SDRAM word {addr:#08x}"
        );
    }
    let refreshes_due = (sim.time() - 15_100) / 585;
    assert!(
        bus.sdram.refreshes() >= 8 + refreshes_due,
        "{} refreshes in {} cycles",
        bus.sdram.refreshes(),
        sim.time()
    );
    assert!(checked > 2000, "only {checked} words read back");
}
