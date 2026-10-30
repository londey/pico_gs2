//! Datasheet timing parameters, converted to clock cycles.

/// Picoseconds per nanosecond.
const PS_PER_NS: u64 = 1_000;

/// The timing a controller must meet, in clock cycles at one clock period.
///
/// [`Timing::any_grade`] takes the slowest value of each parameter across
/// the -5, -6, -6I/-6J/-6L and -75 speed grades (datasheet section 9.5), so
/// a controller that passes against it works with whichever part is fitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// Clock period in picoseconds.
    pub tck_ps: u64,

    /// Shortest clock period allowed at CAS latency 2, in picoseconds.
    pub tck_min_cl2_ps: u64,

    /// Shortest clock period allowed at CAS latency 3, in picoseconds.
    pub tck_min_cl3_ps: u64,

    /// tRC: ACTIVE or REFRESH to ACTIVE or REFRESH, cycles.
    pub rc: u64,

    /// tRAS: ACTIVE to PRECHARGE, cycles.
    pub ras: u64,

    /// tRCD: ACTIVE to READ or WRITE, cycles.
    pub rcd: u64,

    /// tRP: PRECHARGE to ACTIVE or REFRESH, cycles.
    pub rp: u64,

    /// tRRD: ACTIVE in one bank to ACTIVE in another, cycles.
    pub rrd: u64,

    /// tWR: last write data to PRECHARGE, cycles.
    pub wr: u64,

    /// tRSC: MODE REGISTER SET to the next command, cycles.
    pub rsc: u64,

    /// Average refresh interval, tREF / 8192, cycles (rounded down).
    pub refi: u64,

    /// Power-up pause before the first command, cycles (section 7.1).
    pub power_up: u64,
}

impl Timing {
    /// The worst case across all speed grades at a given clock period.
    ///
    /// # Arguments
    ///
    /// * `tck_ps` - The clock period in picoseconds (10 000 for 100 MHz).
    ///
    /// # Returns
    ///
    /// The timing, with each nanosecond parameter rounded up to whole cycles.
    pub fn any_grade(tck_ps: u64) -> Self {
        let cycles = |ns: u64| (ns * PS_PER_NS).div_ceil(tck_ps);
        Self {
            tck_ps,
            // -75 needs 10 ns at CL2; -5 allows 5 ns at CL3, -75 7.5 ns.
            tck_min_cl2_ps: 10_000,
            tck_min_cl3_ps: 7_500,
            rc: cycles(65),
            ras: cycles(45),
            rcd: cycles(20),
            rp: cycles(20),
            rrd: 2,
            wr: 2,
            rsc: 2,
            // 64 ms for 8192 rows (0 to 70 degC).
            refi: 64_000_000 * PS_PER_NS / 8192 / tck_ps,
            power_up: cycles(200_000),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Timing;

    /// At 100 MHz the nanosecond parameters round up to the expected cycles.
    #[test]
    fn any_grade_at_100_mhz() {
        let t = Timing::any_grade(10_000);
        assert_eq!((t.rc, t.ras, t.rcd, t.rp), (7, 5, 2, 2));
        assert_eq!(t.refi, 781);
        assert_eq!(t.power_up, 20_000);
    }
}
