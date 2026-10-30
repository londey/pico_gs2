//! The chip: command decoding, bank state, the read pipeline and the timing
//! checks.

use std::collections::VecDeque;
use std::fmt;

use crate::Timing;

/// Number of banks.
pub const BANKS: usize = 4;

/// Rows per bank (row address A[12:0]).
pub const ROWS: usize = 8192;

/// Columns per row (column address A[8:0]).
pub const COLUMNS: usize = 512;

/// Refreshes the power-up sequence needs (datasheet section 7.1).
const INIT_REFRESHES: u32 = 8;

/// Most violations kept in full; later ones are only counted.
const MAX_KEPT_VIOLATIONS: usize = 100;

/// Address bit A10: auto-precharge on READ/WRITE, all banks on PRECHARGE.
const A10: u16 = 1 << 10;

/// The pins the controller drives, sampled on one rising clock edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pins {
    /// Clock enable.
    pub cke: bool,

    /// Chip select, active low.
    pub cs_n: bool,

    /// Row address strobe, active low.
    pub ras_n: bool,

    /// Column address strobe, active low.
    pub cas_n: bool,

    /// Write enable, active low.
    pub we_n: bool,

    /// Bank address, BA[1:0].
    pub ba: u8,

    /// Address, A[12:0].
    pub a: u16,

    /// Data mask: bit 0 is LDQM (DQ[7:0]), bit 1 is UDQM (DQ[15:8]).
    pub dqm: u8,

    /// What the controller drives on DQ, or `None` while it is high
    /// impedance.
    pub dq: Option<u16>,
}

impl Pins {
    /// A NOP with CKE high, DQM low and DQ released.
    ///
    /// # Returns
    ///
    /// The pins for a NOP.
    pub fn nop() -> Self {
        Self {
            cke: true,
            cs_n: false,
            ras_n: true,
            cas_n: true,
            we_n: true,
            ba: 0,
            a: 0,
            dqm: 0,
            dq: None,
        }
    }
}

/// What the chip drives on DQ between one rising edge and the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dq {
    /// The data. Bits the chip doesn't drive are 0.
    pub value: u16,

    /// One bit per DQ pin: 1 where the chip drives it.
    pub driven: u16,
}

impl Dq {
    /// The word, if the chip drives all sixteen bits.
    ///
    /// # Returns
    ///
    /// `Some` word when both bytes are driven, otherwise `None`.
    pub fn word(self) -> Option<u16> {
        (self.driven == 0xFFFF).then_some(self.value)
    }
}

/// A breach of the datasheet, or use of a feature the model doesn't cover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// The clock cycle (rising edge, counted from 0 at power-up).
    pub cycle: u64,

    /// What went wrong.
    pub message: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cycle {}: {}", self.cycle, self.message)
    }
}

/// A command, decoded from CS#, RAS#, CAS# and WE#.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Inhibit,
    Nop,
    Active,
    Read,
    Write,
    BurstStop,
    Precharge,
    Refresh,
    ModeRegisterSet,
}

impl Command {
    /// Decodes the command pins (datasheet section 8, command truth table).
    fn decode(pins: &Pins) -> Self {
        if pins.cs_n {
            return Self::Inhibit;
        }
        match (pins.ras_n, pins.cas_n, pins.we_n) {
            (true, true, true) => Self::Nop,
            (false, true, true) => Self::Active,
            (true, false, true) => Self::Read,
            (true, false, false) => Self::Write,
            (true, true, false) => Self::BurstStop,
            (false, true, false) => Self::Precharge,
            (false, false, true) => Self::Refresh,
            (false, false, false) => Self::ModeRegisterSet,
        }
    }

    /// The command's name, for messages.
    fn name(self) -> &'static str {
        match self {
            Self::Inhibit => "COMMAND INHIBIT",
            Self::Nop => "NOP",
            Self::Active => "ACTIVE",
            Self::Read => "READ",
            Self::Write => "WRITE",
            Self::BurstStop => "BURST STOP",
            Self::Precharge => "PRECHARGE",
            Self::Refresh => "REFRESH",
            Self::ModeRegisterSet => "MODE REGISTER SET",
        }
    }
}

/// The mode register fields the model supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mode {
    /// CAS latency, 2 or 3 cycles.
    cas_latency: u64,

    /// Burst length: 1, 2, 4 or 8.
    burst_length: u64,

    /// A9 set: writes are single-location whatever the burst length.
    single_write: bool,
}

/// One bank's state and the times of its last commands.
#[derive(Clone, Copy, Debug, Default)]
struct Bank {
    /// The open row, or `None` while the bank is idle (precharged).
    row: Option<usize>,

    /// Cycle of the last ACTIVE.
    activated: Option<u64>,

    /// Cycle of the last PRECHARGE that closed a row.
    precharged: Option<u64>,

    /// Cycle of the last write data beat.
    written: Option<u64>,
}

/// One word of a read burst, due on DQ in a given cycle.
#[derive(Clone, Copy, Debug)]
struct ReadBeat {
    cycle: u64,
    bank: usize,
    row: usize,
    column: usize,
}

/// A write burst in progress.
#[derive(Clone, Copy, Debug)]
struct WriteBurst {
    bank: usize,
    row: usize,
    column: usize,
    beat: u64,
    length: u64,
}

/// The W9825G6KH.
pub struct Sdram {
    /// The timing the controller must meet.
    timing: Timing,

    /// The current cycle: rising edges since power-up.
    cycle: u64,

    /// The array, indexed by bank, then row, then column.
    memory: Vec<u16>,

    /// Per-bank state.
    banks: [Bank; BANKS],

    /// The mode register, once set.
    mode: Option<Mode>,

    /// A PRECHARGE ALL has followed the power-up pause.
    precharged_all: bool,

    /// REFRESH commands since that PRECHARGE ALL, while initialising.
    init_refreshes: u32,

    /// Cycle and bank of the last ACTIVE in any bank (for tRRD).
    last_active: Option<(u64, usize)>,

    /// Cycle of the last REFRESH.
    last_refresh: Option<u64>,

    /// Cycle of the last MODE REGISTER SET.
    last_mode_set: Option<u64>,

    /// The refresh overdue violation has been reported since the last
    /// REFRESH.
    refresh_overdue_reported: bool,

    /// The DQM-not-high-during-pause violation has been reported.
    pause_dqm_reported: bool,

    /// REFRESH commands since power-up.
    refreshes: u64,

    /// Read data waiting for its cycle on DQ, in cycle order.
    reads: VecDeque<ReadBeat>,

    /// The write burst taking data, if any.
    write: Option<WriteBurst>,

    /// DQM sampled one and two cycles ago (read masking has latency 2).
    dqm_history: [u8; 2],

    /// The first violations, in order.
    violations: Vec<Violation>,

    /// All violations, including those not kept.
    violation_count: u64,
}

impl Sdram {
    /// A chip at power-up: contents zero, mode register unset.
    ///
    /// # Arguments
    ///
    /// * `timing` - The timing to check the controller against.
    ///
    /// # Returns
    ///
    /// The model, at cycle 0.
    pub fn new(timing: Timing) -> Self {
        Self {
            timing,
            cycle: 0,
            memory: vec![0; BANKS * ROWS * COLUMNS],
            banks: [Bank::default(); BANKS],
            mode: None,
            precharged_all: false,
            init_refreshes: 0,
            last_active: None,
            last_refresh: None,
            last_mode_set: None,
            refresh_overdue_reported: false,
            pause_dqm_reported: false,
            refreshes: 0,
            reads: VecDeque::new(),
            write: None,
            dqm_history: [0; 2],
            violations: Vec::new(),
            violation_count: 0,
        }
    }

    /// One rising edge of the SDRAM clock.
    ///
    /// # Arguments
    ///
    /// * `pins` - The controller's pins, as sampled on this edge.
    ///
    /// # Returns
    ///
    /// What the chip drives on DQ from this edge until the next.
    pub fn clock(&mut self, pins: &Pins) -> Dq {
        let dq = self.drive();
        self.command(pins);
        self.write_beat(pins);
        if dq.driven != 0 && pins.dq.is_some() {
            self.violation(
                "bus contention: the controller drives DQ while the chip outputs read data",
            );
        }
        self.check_refresh_due();
        self.dqm_history = [pins.dqm, self.dqm_history[0]];
        self.cycle += 1;
        dq
    }

    /// The number of rising edges so far.
    ///
    /// # Returns
    ///
    /// The cycle count.
    pub fn cycle(&self) -> u64 {
        self.cycle
    }

    /// Whether the power-up sequence is complete and the chip accepts
    /// ACTIVE.
    ///
    /// # Returns
    ///
    /// `true` once PRECHARGE ALL, eight REFRESH and MODE REGISTER SET have
    /// followed the power-up pause.
    pub fn ready(&self) -> bool {
        self.precharged_all && self.init_refreshes >= INIT_REFRESHES && self.mode.is_some()
    }

    /// The REFRESH commands since power-up.
    ///
    /// # Returns
    ///
    /// The count.
    pub fn refreshes(&self) -> u64 {
        self.refreshes
    }

    /// The first violations recorded, in order (up to 100).
    ///
    /// # Returns
    ///
    /// The violations.
    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }

    /// The number of violations, including any beyond those kept.
    ///
    /// # Returns
    ///
    /// The count.
    pub fn violation_count(&self) -> u64 {
        self.violation_count
    }

    /// Reads a word directly from the array, bypassing the pins.
    ///
    /// # Arguments
    ///
    /// * `bank` - Bank, below [`BANKS`].
    /// * `row` - Row, below [`ROWS`].
    /// * `column` - Column, below [`COLUMNS`].
    ///
    /// # Returns
    ///
    /// The stored word.
    ///
    /// # Panics
    ///
    /// If an index is out of range.
    pub fn peek(&self, bank: usize, row: usize, column: usize) -> u16 {
        self.memory[index(bank, row, column)]
    }

    /// Writes a word directly into the array, bypassing the pins.
    ///
    /// # Arguments
    ///
    /// * `bank` - Bank, below [`BANKS`].
    /// * `row` - Row, below [`ROWS`].
    /// * `column` - Column, below [`COLUMNS`].
    /// * `value` - The word to store.
    ///
    /// # Panics
    ///
    /// If an index is out of range.
    pub fn poke(&mut self, bank: usize, row: usize, column: usize, value: u16) {
        self.memory[index(bank, row, column)] = value;
    }

    /// Records a violation in the current cycle.
    fn violation(&mut self, message: impl Into<String>) {
        self.violation_count += 1;
        if self.violations.len() < MAX_KEPT_VIOLATIONS {
            self.violations.push(Violation {
                cycle: self.cycle,
                message: message.into(),
            });
        }
    }

    /// Records a violation unless `ok`.
    fn require(&mut self, ok: bool, message: impl FnOnce() -> String) {
        if !ok {
            self.violation(message());
        }
    }

    /// Cycles from `then` to now, or `u64::MAX` if it never happened.
    fn since(&self, then: Option<u64>) -> u64 {
        then.map_or(u64::MAX, |t| self.cycle - t)
    }

    /// The CAS latency, or 0 before the mode register is set.
    fn cas_latency(&self) -> u64 {
        self.mode.map_or(0, |m| m.cas_latency)
    }

    /// Takes the read data due this cycle, masked by the DQM sampled two
    /// edges ago.
    fn drive(&mut self) -> Dq {
        let Some(beat) = self.reads.front().copied() else {
            return Dq::default();
        };
        if beat.cycle != self.cycle {
            return Dq::default();
        }
        self.reads.pop_front();
        let mask = self.dqm_history[1];
        let driven =
            if mask & 1 == 0 { 0x00FF } else { 0 } | if mask & 2 == 0 { 0xFF00 } else { 0 };
        let value = self.memory[index(beat.bank, beat.row, beat.column)] & driven;
        Dq { value, driven }
    }

    /// Decodes and executes the command on the pins.
    fn command(&mut self, pins: &Pins) {
        let command = Command::decode(pins);
        if !pins.cke {
            self.violation("CKE low: power-down and self refresh aren't modelled");
            return;
        }
        if self.cycle < self.timing.power_up {
            if pins.dqm != 0b11 && !self.pause_dqm_reported {
                self.pause_dqm_reported = true;
                self.violation("DQM must be held high during the power-up pause (section 7.1)");
            }
            self.require(matches!(command, Command::Inhibit | Command::Nop), || {
                format!("{} during the 200 us power-up pause", command.name())
            });
            return;
        }
        match command {
            Command::Inhibit | Command::Nop => {}
            Command::Active => self.active(pins),
            Command::Read | Command::Write => self.access(command, pins),
            // Section 7.11: only legal to end a full-page burst, which
            // isn't modelled.
            Command::BurstStop => self.violation("BURST STOP is only legal with full-page bursts"),
            Command::Precharge => self.precharge(pins),
            Command::Refresh => self.refresh(),
            Command::ModeRegisterSet => self.mode_register_set(pins),
        }
    }

    /// Checks tRSC after a MODE REGISTER SET.
    fn require_mode_settled(&mut self, command: Command) {
        let ok = self.since(self.last_mode_set) >= self.timing.rsc;
        self.require(ok, || {
            format!("tRSC: {} too soon after MODE REGISTER SET", command.name())
        });
    }

    /// ACTIVE: opens a row.
    fn active(&mut self, pins: &Pins) {
        let b = usize::from(pins.ba & 3);
        let row = usize::from(pins.a & 0x1FFF);
        let bank = self.banks[b];
        self.require(self.ready(), || {
            "ACTIVE before initialisation (PRECHARGE ALL, 8 REFRESH, MODE REGISTER SET) is complete"
                .into()
        });
        if let Some(open) = bank.row {
            self.violation(format!(
                "ACTIVE to bank {b}, which already has row {open} open"
            ));
        }
        self.require(self.since(bank.precharged) >= self.timing.rp, || {
            format!("tRP: ACTIVE to bank {b} too soon after PRECHARGE")
        });
        self.require(self.since(bank.activated) >= self.timing.rc, || {
            format!("tRC: ACTIVE to bank {b} too soon after its last ACTIVE")
        });
        if let Some((cycle, other)) = self.last_active {
            if other != b {
                self.require(self.since(Some(cycle)) >= self.timing.rrd, || {
                    format!("tRRD: ACTIVE to bank {b} too soon after ACTIVE to bank {other}")
                });
            }
        }
        self.require(self.since(self.last_refresh) >= self.timing.rc, || {
            "tRC: ACTIVE too soon after REFRESH".into()
        });
        self.require_mode_settled(Command::Active);
        self.banks[b].row = Some(row);
        self.banks[b].activated = Some(self.cycle);
        self.last_active = Some((self.cycle, b));
    }

    /// READ or WRITE: starts a burst in an open row.
    fn access(&mut self, command: Command, pins: &Pins) {
        let b = usize::from(pins.ba & 3);
        let column = usize::from(pins.a & 0x1FF);
        let Some(mode) = self.mode.filter(|_| self.ready()) else {
            self.violation(format!(
                "{} before initialisation is complete",
                command.name()
            ));
            return;
        };
        if pins.a & A10 != 0 {
            self.violation(format!(
                "{} with auto-precharge (A10 high) isn't modelled",
                command.name()
            ));
        }
        let bank = self.banks[b];
        let Some(row) = bank.row else {
            self.violation(format!(
                "{} to bank {b}, which has no open row",
                command.name()
            ));
            return;
        };
        self.require(self.since(bank.activated) >= self.timing.rcd, || {
            format!("tRCD: {} to bank {b} too soon after ACTIVE", command.name())
        });
        self.require_mode_settled(command);
        self.terminate_bursts(None);
        if command == Command::Read {
            for beat in 0..mode.burst_length {
                self.reads.push_back(ReadBeat {
                    cycle: self.cycle + mode.cas_latency + beat,
                    bank: b,
                    row,
                    column: burst_column(column, beat, mode.burst_length),
                });
            }
        } else {
            let length = if mode.single_write {
                1
            } else {
                mode.burst_length
            };
            self.write = Some(WriteBurst {
                bank: b,
                row,
                column,
                beat: 0,
                length,
            });
        }
    }

    /// Ends bursts, in one bank or all: read data already in the CAS
    /// latency pipeline still comes out, later beats don't; a write burst
    /// takes no more data.
    fn terminate_bursts(&mut self, bank: Option<usize>) {
        let cutoff = self.cycle + self.cas_latency();
        let hit = |b: usize| bank.is_none_or(|x| x == b);
        self.reads.retain(|r| !(hit(r.bank) && r.cycle >= cutoff));
        if self.write.is_some_and(|w| hit(w.bank)) {
            self.write = None;
        }
    }

    /// PRECHARGE: closes the row in one bank, or in all with A10 high.
    fn precharge(&mut self, pins: &Pins) {
        let all = pins.a & A10 != 0;
        let selected = usize::from(pins.ba & 3);
        for b in 0..BANKS {
            if !all && b != selected {
                continue;
            }
            let bank = self.banks[b];
            if bank.row.is_none() {
                continue;
            }
            self.require(self.since(bank.activated) >= self.timing.ras, || {
                format!("tRAS: PRECHARGE of bank {b} too soon after ACTIVE")
            });
            self.require(self.since(bank.written) >= self.timing.wr, || {
                format!("tWR: PRECHARGE of bank {b} too soon after write data")
            });
            self.banks[b].row = None;
            self.banks[b].precharged = Some(self.cycle);
        }
        self.require_mode_settled(Command::Precharge);
        self.terminate_bursts(if all { None } else { Some(selected) });
        if all {
            self.precharged_all = true;
        }
    }

    /// Checks that every bank is idle and past tRP, for REFRESH and MODE
    /// REGISTER SET.
    fn require_all_idle(&mut self, command: Command) {
        self.require(self.precharged_all, || {
            format!("{} before PRECHARGE ALL", command.name())
        });
        for b in 0..BANKS {
            let bank = self.banks[b];
            if let Some(row) = bank.row {
                self.violation(format!(
                    "{} while bank {b} has row {row} open",
                    command.name()
                ));
            }
            self.require(self.since(bank.precharged) >= self.timing.rp, || {
                format!(
                    "tRP: {} too soon after PRECHARGE of bank {b}",
                    command.name()
                )
            });
        }
        self.require(self.since(self.last_refresh) >= self.timing.rc, || {
            format!("tRC: {} too soon after REFRESH", command.name())
        });
        self.require_mode_settled(command);
    }

    /// REFRESH (auto refresh).
    fn refresh(&mut self) {
        self.require_all_idle(Command::Refresh);
        let since_active = self.since(self.last_active.map(|(cycle, _)| cycle));
        self.require(since_active >= self.timing.rc, || {
            "tRC: REFRESH too soon after ACTIVE".into()
        });
        if !self.ready() {
            self.init_refreshes += 1;
        }
        self.refreshes += 1;
        self.last_refresh = Some(self.cycle);
        self.refresh_overdue_reported = false;
    }

    /// Reports, once per gap, a refresh that is overdue.
    ///
    /// Refresh must average one REFRESH per tREF / 8192. The check allows
    /// eight to be postponed (as JEDEC SDRAM allows), so a gap of more than
    /// nine intervals is a violation.
    fn check_refresh_due(&mut self) {
        if !self.ready() || self.refresh_overdue_reported {
            return;
        }
        let limit = 9 * self.timing.refi;
        if self.since(self.last_refresh) > limit {
            self.refresh_overdue_reported = true;
            self.violation(format!(
                "refresh overdue: no REFRESH for more than {limit} cycles"
            ));
        }
    }

    /// MODE REGISTER SET.
    fn mode_register_set(&mut self, pins: &Pins) {
        self.require_all_idle(Command::ModeRegisterSet);
        self.last_mode_set = Some(self.cycle);
        let a = pins.a;
        let burst_length = match a & 0b111 {
            0 => 1,
            1 => 2,
            2 => 4,
            3 => 8,
            7 => return self.violation("full-page bursts aren't modelled"),
            _ => return self.violation(format!("reserved burst length code {}", a & 0b111)),
        };
        if a & (1 << 3) != 0 {
            return self.violation("interleaved bursts aren't modelled");
        }
        let cas_latency = u64::from((a >> 4) & 0b111);
        let tck_min = match cas_latency {
            2 => self.timing.tck_min_cl2_ps,
            3 => self.timing.tck_min_cl3_ps,
            _ => return self.violation(format!("reserved CAS latency {cas_latency}")),
        };
        if (a >> 7) & 0b11 != 0 {
            return self.violation("A[8:7] must be 0 (test mode is reserved)");
        }
        let tck = self.timing.tck_ps;
        self.require(tck >= tck_min, || {
            format!("CAS latency {cas_latency} needs tCK >= {tck_min} ps; the clock is {tck} ps")
        });
        self.mode = Some(Mode {
            cas_latency,
            burst_length,
            single_write: a & (1 << 9) != 0,
        });
    }

    /// Takes this cycle's beat of a write burst from DQ, masked by DQM.
    fn write_beat(&mut self, pins: &Pins) {
        let Some(mut burst) = self.write else {
            return;
        };
        let at = index(
            burst.bank,
            burst.row,
            burst_column(burst.column, burst.beat, burst.length),
        );
        let keep =
            if pins.dqm & 1 == 0 { 0 } else { 0x00FF } | if pins.dqm & 2 == 0 { 0 } else { 0xFF00 };
        if keep != 0xFFFF {
            match pins.dq {
                Some(data) => self.memory[at] = (self.memory[at] & keep) | (data & !keep),
                None => self.violation("write data beat with DQ not driven"),
            }
        }
        self.banks[burst.bank].written = Some(self.cycle);
        burst.beat += 1;
        self.write = (burst.beat < burst.length).then_some(burst);
    }
}

/// The array index of a word.
fn index(bank: usize, row: usize, column: usize) -> usize {
    assert!(
        bank < BANKS && row < ROWS && column < COLUMNS,
        "address out of range"
    );
    (bank * ROWS + row) * COLUMNS + column
}

/// The column of beat `beat` of a sequential burst of `length` starting at
/// `start`: the burst wraps within its aligned block (datasheet section 7.12).
fn burst_column(start: usize, beat: u64, length: u64) -> usize {
    let mask = usize::try_from(length - 1).unwrap_or(0);
    let beat = usize::try_from(beat).unwrap_or(0);
    (start & !mask) | ((start + beat) & mask)
}
