//! Tests of the SDRAM model against the W9825G6KH datasheet: correct use
//! stores and returns data with the right latency, and each timing rule a
//! controller can break is reported.

use sdram_model::{Dq, Pins, Sdram, Timing};

/// Clock period of the GPU's SDRAM clock (100 MHz), in picoseconds.
const TCK_PS: u64 = 10_000;

/// Mode register: burst length 1, sequential, CAS latency 2.
const MODE_CL2_BL1: u16 = 0x020;

/// Mode register: burst length 1, sequential, CAS latency 3.
const MODE_CL3_BL1: u16 = 0x030;

/// Mode register: burst length 4, sequential, CAS latency 2.
const MODE_CL2_BL4: u16 = 0x022;

/// A10: all banks on PRECHARGE.
const ALL_BANKS: u16 = 1 << 10;

/// Drives the model one command per cycle.
struct Bus {
    /// The chip.
    sdram: Sdram,

    /// The model's timing, for waits.
    timing: Timing,

    /// DQM to drive with each command.
    dqm: u8,
}

impl Bus {
    /// A chip at power-up, clocked at 100 MHz.
    fn new() -> Self {
        Self::with_timing(Timing::any_grade(TCK_PS))
    }

    /// A chip at power-up with the given timing.
    fn with_timing(timing: Timing) -> Self {
        Self {
            sdram: Sdram::new(timing),
            timing,
            dqm: 0,
        }
    }

    /// Drives one cycle with the given command pins and DQ.
    fn drive(
        &mut self,
        (ras_n, cas_n, we_n): (bool, bool, bool),
        ba: u8,
        a: u16,
        dq: Option<u16>,
    ) -> Dq {
        let pins = Pins {
            ras_n,
            cas_n,
            we_n,
            ba,
            a,
            dqm: self.dqm,
            dq,
            ..Pins::nop()
        };
        self.sdram.clock(&pins)
    }

    /// Drives `n` NOPs, returning what the chip drove on each.
    fn nops(&mut self, n: u64) -> Vec<Dq> {
        (0..n)
            .map(|_| self.drive((true, true, true), 0, 0, None))
            .collect()
    }

    /// ACTIVE.
    fn active(&mut self, ba: u8, row: u16) -> Dq {
        self.drive((false, true, true), ba, row, None)
    }

    /// READ (no auto-precharge).
    fn read(&mut self, ba: u8, column: u16) -> Dq {
        self.drive((true, false, true), ba, column, None)
    }

    /// WRITE (no auto-precharge), driving the first data beat.
    fn write(&mut self, ba: u8, column: u16, data: u16) -> Dq {
        self.drive((true, false, false), ba, column, Some(data))
    }

    /// PRECHARGE of one bank, or all with `a` = [`ALL_BANKS`].
    fn precharge(&mut self, ba: u8, a: u16) -> Dq {
        self.drive((false, true, false), ba, a, None)
    }

    /// REFRESH.
    fn refresh(&mut self) -> Dq {
        self.drive((false, false, true), 0, 0, None)
    }

    /// MODE REGISTER SET.
    fn mode_register_set(&mut self, mode: u16) -> Dq {
        self.drive((false, false, false), 0, mode, None)
    }

    /// The datasheet's power-up sequence (section 7.1): the pause with DQM
    /// high, PRECHARGE ALL, eight REFRESH, MODE REGISTER SET, each followed
    /// by its wait.
    fn init(&mut self, mode: u16) {
        self.dqm = 0b11;
        self.nops(self.timing.power_up);
        self.dqm = 0;
        self.precharge(0, ALL_BANKS);
        self.nops(self.timing.rp - 1);
        for _ in 0..8 {
            self.refresh();
            self.nops(self.timing.rc - 1);
        }
        self.mode_register_set(mode);
        self.nops(self.timing.rsc - 1);
        assert!(
            self.sdram.ready(),
            "init didn't complete: {:?}",
            self.sdram.violations()
        );
    }

    /// Asserts that no violation was reported.
    fn assert_clean(&self) {
        assert!(
            self.sdram.violations().is_empty(),
            "violations: {:?}",
            self.sdram.violations()
        );
    }

    /// Asserts that a violation mentioning `what` was reported.
    fn assert_reported(&self, what: &str) {
        assert!(
            self.sdram
                .violations()
                .iter()
                .any(|v| v.message.contains(what)),
            "expected a violation mentioning {what:?}, got {:?}",
            self.sdram.violations()
        );
    }
}

/// A word written at CAS latency 2 reads back two cycles after its READ,
/// and the bus is released on the cycles around it.
#[test]
fn write_then_read_at_cas_latency_2() {
    let mut bus = Bus::new();
    bus.init(MODE_CL2_BL1);
    bus.active(1, 100);
    bus.nops(bus.timing.rcd - 1);
    bus.write(1, 5, 0xBEEF);
    bus.nops(bus.timing.ras);
    bus.precharge(1, 0);
    bus.nops(bus.timing.rp - 1);
    assert_eq!(bus.sdram.peek(1, 100, 5), 0xBEEF);

    bus.active(1, 100);
    bus.nops(bus.timing.rcd - 1);
    let issued = bus.read(1, 5);
    let after = bus.nops(3);
    assert_eq!(issued.driven, 0);
    assert_eq!(after[0].driven, 0);
    assert_eq!(after[1].word(), Some(0xBEEF));
    assert_eq!(after[2].driven, 0);
    bus.assert_clean();
}

/// At CAS latency 3 read data arrives three cycles after the READ.
#[test]
fn read_at_cas_latency_3() {
    let mut bus = Bus::new();
    bus.init(MODE_CL3_BL1);
    bus.sdram.poke(2, 7, 300, 0x1234);
    bus.active(2, 7);
    bus.nops(bus.timing.rcd - 1);
    bus.read(2, 300);
    let after = bus.nops(3);
    assert_eq!(after[1].driven, 0);
    assert_eq!(after[2].word(), Some(0x1234));
    bus.assert_clean();
}

/// Back-to-back single-word READs to consecutive columns stream one word
/// per cycle.
#[test]
fn back_to_back_reads_stream() {
    let mut bus = Bus::new();
    bus.init(MODE_CL2_BL1);
    for column in 0..8 {
        bus.sdram
            .poke(0, 3, column, 0x100 + u16::try_from(column).unwrap());
    }
    bus.active(0, 3);
    bus.nops(bus.timing.rcd - 1);
    let mut seen = Vec::new();
    for column in 0..8 {
        seen.push(bus.read(0, column));
    }
    seen.extend(bus.nops(2));
    let words: Vec<_> = seen.iter().filter_map(|d| d.word()).collect();
    assert_eq!(words, (0x100..0x108).collect::<Vec<u16>>());
    assert_eq!(
        seen[2].word(),
        Some(0x100),
        "first word two cycles after its READ"
    );
    bus.assert_clean();
}

/// A sequential burst of four wraps within its aligned block of four
/// columns (datasheet section 7.12, table 2).
#[test]
fn burst_of_four_wraps_within_its_block() {
    let mut bus = Bus::new();
    bus.init(MODE_CL2_BL4);
    for column in 4..8 {
        bus.sdram.poke(0, 0, column, u16::try_from(column).unwrap());
    }
    bus.active(0, 0);
    bus.nops(bus.timing.rcd - 1);
    bus.read(0, 6);
    let words: Vec<_> = bus.nops(5).iter().filter_map(|d| d.word()).collect();
    assert_eq!(words, vec![6, 7, 4, 5]);
    bus.assert_clean();
}

/// DQM masks a write byte in the same cycle and read output two cycles
/// later.
#[test]
fn dqm_masks_write_bytes_and_read_output() {
    let mut bus = Bus::new();
    bus.init(MODE_CL2_BL1);
    bus.sdram.poke(0, 1, 2, 0xAAAA);
    bus.active(0, 1);
    bus.nops(bus.timing.rcd - 1);
    bus.dqm = 0b01;
    bus.write(0, 2, 0x5555);
    bus.dqm = 0;
    assert_eq!(bus.sdram.peek(0, 1, 2), 0x55AA, "LDQM keeps the low byte");

    bus.dqm = 0b10;
    bus.read(0, 2);
    bus.dqm = 0;
    let after = bus.nops(2);
    assert_eq!(
        after[1],
        Dq {
            value: 0x00AA,
            driven: 0x00FF
        }
    );
    bus.assert_clean();
}

/// Any command other than NOP during the 200 us pause is reported.
#[test]
fn reports_command_during_power_up_pause() {
    let mut bus = Bus::new();
    bus.dqm = 0b11;
    bus.nops(10);
    bus.precharge(0, ALL_BANKS);
    bus.assert_reported("power-up pause");
}

/// ACTIVE before the eight initial refreshes is reported.
#[test]
fn reports_active_before_initialisation() {
    let mut bus = Bus::new();
    bus.dqm = 0b11;
    bus.nops(bus.timing.power_up);
    bus.precharge(0, ALL_BANKS);
    bus.nops(bus.timing.rp);
    bus.mode_register_set(MODE_CL2_BL1);
    bus.nops(bus.timing.rsc);
    bus.active(0, 0);
    bus.assert_reported("before initialisation");
}

/// READ one cycle after ACTIVE breaks tRCD.
#[test]
fn reports_trcd() {
    let mut bus = Bus::new();
    bus.init(MODE_CL2_BL1);
    bus.active(0, 0);
    bus.read(0, 0);
    bus.assert_reported("tRCD");
}

/// PRECHARGE too soon after ACTIVE breaks tRAS.
#[test]
fn reports_tras() {
    let mut bus = Bus::new();
    bus.init(MODE_CL2_BL1);
    bus.active(0, 0);
    bus.nops(bus.timing.rcd - 1);
    bus.read(0, 0);
    bus.precharge(0, 0);
    bus.assert_reported("tRAS");
}

/// PRECHARGE one cycle after the last write data breaks tWR.
#[test]
fn reports_twr() {
    let mut bus = Bus::new();
    bus.init(MODE_CL2_BL1);
    bus.active(0, 0);
    bus.nops(bus.timing.ras);
    bus.write(0, 0, 1);
    bus.precharge(0, 0);
    bus.assert_reported("tWR");
}

/// ACTIVE one cycle after PRECHARGE breaks tRP.
#[test]
fn reports_trp() {
    let mut bus = Bus::new();
    bus.init(MODE_CL2_BL1);
    bus.active(0, 0);
    bus.nops(bus.timing.rc - 1);
    bus.precharge(0, 0);
    bus.active(0, 1);
    bus.assert_reported("tRP");
}

/// ACTIVE in two banks on consecutive cycles breaks tRRD.
#[test]
fn reports_trrd() {
    let mut bus = Bus::new();
    bus.init(MODE_CL2_BL1);
    bus.active(0, 0);
    bus.active(1, 0);
    bus.assert_reported("tRRD");
}

/// REFRESH with a row open is reported.
#[test]
fn reports_refresh_with_open_bank() {
    let mut bus = Bus::new();
    bus.init(MODE_CL2_BL1);
    bus.active(3, 9);
    bus.nops(bus.timing.rc);
    bus.refresh();
    bus.assert_reported("bank 3 has row 9 open");
}

/// Going more than nine refresh intervals without a REFRESH is reported.
#[test]
fn reports_overdue_refresh() {
    let mut bus = Bus::new();
    bus.init(MODE_CL2_BL1);
    bus.nops(9 * bus.timing.refi - 50);
    bus.assert_clean();
    bus.nops(100);
    bus.assert_reported("refresh overdue");
}

/// Driving DQ while the chip outputs read data is reported.
#[test]
fn reports_bus_contention() {
    let mut bus = Bus::new();
    bus.init(MODE_CL2_BL1);
    bus.active(0, 0);
    bus.nops(bus.timing.rcd - 1);
    bus.read(0, 0);
    bus.nops(1);
    bus.write(0, 1, 0);
    bus.assert_reported("bus contention");
}

/// CAS latency 2 at 133 MHz is too fast for the slowest grade.
#[test]
fn reports_clock_too_fast_for_cas_latency_2() {
    let mut bus = Bus::with_timing(Timing::any_grade(7_500));
    bus.init(MODE_CL2_BL1);
    bus.assert_reported("CAS latency 2 needs tCK");
}
