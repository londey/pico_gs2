//! Test of the host link (`hw/link.snt`) on its own: bytes on its receive
//! line at 3 Mbaud become transactions in its FIFO, in order; DTR# high
//! resets it, dropping a partial transaction and emptying the FIFO; it
//! returns a credit byte for every 64 transactions taken and sends read
//! replies whole; and it flags a FIFO overflow and a bad stop bit.

use gs_proto::{reg, Reply, ReplyDecoder, Transaction, ID_VALUE, TAG_REPLY};
use host_link_sim::{link__HostLink as HostLink, Inputs, Outputs};
use tb::{UartRx, UartTx, BIT_CYCLES};

/// The link with a host's UART on each side of it.
struct Rig {
    /// The link.
    sim: HostLink,

    /// The host's transmitter: the link's receive line.
    tx: UartTx,

    /// The host's receiver, on the link's transmit line.
    rx: UartRx,

    /// The link's transmit line, from the last cycle.
    line: u8,

    /// DTR#: 1 holds the link in reset.
    dtr_n: u8,

    /// Bytes the link sent.
    sent: Vec<u8>,

    /// Transactions taken from the FIFO: address byte and data.
    taken: Vec<(u8, u64)>,

    /// Whether the head can be taken this cycle: the FIFO had an entry last
    /// cycle and wasn't popped (its memory reads a cycle behind its
    /// address, so a new or moved head is valid a cycle later, as the
    /// command parser's FETCH then DECODE allow for).
    ready: bool,
}

impl Rig {
    /// A link out of reset, connected (DTR# low).
    fn new() -> Rig {
        Rig {
            sim: HostLink::new(),
            tx: UartTx::default(),
            rx: UartRx::default(),
            line: 1,
            dtr_n: 0,
            sent: Vec::new(),
            taken: Vec::new(),
            ready: false,
        }
    }

    /// Queues a transaction's bytes.
    fn send(&mut self, t: Transaction) {
        let mut b = [0; 9];
        let n = t.encode(&mut b);
        for &x in &b[..n] {
            self.tx.push(x);
        }
    }

    /// One cycle, the receive line from the host's UART. `take`: pop the
    /// FIFO's head when it is valid. `reply`: offer a read's value.
    fn step(&mut self, take: bool, reply: Option<u64>) -> Outputs {
        let rx = self.tx.line();
        self.step_line(rx, take, reply)
    }

    /// One cycle with the receive line given.
    fn step_line(&mut self, rx: u8, take: bool, reply: Option<u64>) -> Outputs {
        let mut inputs = Inputs {
            rx,
            dtr_n: self.dtr_n,
            pop: 0,
            reply_is_some: u8::from(reply.is_some()),
            reply_payload: reply.unwrap_or(0),
        };
        let now = self.sim.eval(&inputs);
        let pop = take && now.out_avail == 1 && self.ready;
        if pop {
            self.taken.push((now.out_head_addr, now.out_head_data));
        }
        inputs.pop = u8::from(pop);
        self.ready = now.out_avail == 1 && !pop;
        let out = self.sim.step(&inputs);
        if let Some(b) = self.rx.clock(self.line) {
            self.sent.push(b);
        }
        self.line = out.out_tx;
        out
    }

    /// Runs until the host's transmitter is idle and a frame time more.
    fn drain(&mut self, take: bool) -> Outputs {
        let mut out = self.step(take, None);
        while !self.tx.idle() {
            out = self.step(take, None);
        }
        for _ in 0..20 * BIT_CYCLES {
            out = self.step(take, None);
        }
        out
    }
}

/// Writes and reads arrive in the FIFO as sent, in order.
#[test]
fn transactions_arrive_in_order() {
    let mut rig = Rig::new();
    let txns = [
        Transaction::write(reg::ATTR0, 0x0123_4567_89AB_CDEF),
        Transaction::Read { addr: reg::ID },
        Transaction::write(reg::FRAME, 0xFFFF_0000_0000_0007),
        Transaction::Read { addr: reg::STATUS },
    ];
    for t in txns {
        rig.send(t);
    }
    let out = rig.drain(true);
    let want: Vec<_> = txns.iter().map(|t| (t.addr_byte(), t.data())).collect();
    assert_eq!(rig.taken, want);
    assert_eq!(out.out_avail, 0);
    assert_eq!(out.out_free, 512);
    assert_eq!((out.out_overflow, out.out_framing), (0, 0));
}

/// DTR# high in the middle of a transaction drops it and empties the FIFO;
/// the next transaction after DTR# falls again arrives whole.
#[test]
fn dtr_resets_the_link() {
    let mut rig = Rig::new();
    rig.send(Transaction::Read { addr: reg::ID });
    rig.drain(false);
    assert_eq!(rig.sim.eval(&Inputs::default()).out_avail, 1);
    // Five bytes of a write, then the host goes.
    let mut b = [0; 9];
    Transaction::write(reg::ATTR0, u64::MAX).encode(&mut b);
    for &x in &b[..5] {
        rig.tx.push(x);
    }
    let out = rig.drain(false);
    assert_eq!(out.out_free, 511, "the partial write isn't in the FIFO");
    rig.dtr_n = 1;
    for _ in 0..10 {
        rig.step(false, None);
    }
    let out = rig.step(false, None);
    assert_eq!(
        (out.out_avail, out.out_enabled, out.out_free),
        (0, 0, 512),
        "reset"
    );
    rig.dtr_n = 0;
    let write = Transaction::write(reg::FRAME, 0x1234);
    rig.send(write);
    rig.drain(true);
    assert_eq!(rig.taken, [(write.addr_byte(), write.data())]);
}

/// A credit byte for every 64 transactions taken, and a read reply as its
/// tag and 8 bytes, never split by a credit.
#[test]
fn credits_and_replies_go_back() {
    let mut rig = Rig::new();
    for _ in 0..130 {
        rig.send(Transaction::Read { addr: reg::ID });
    }
    // Offer a reply while the credits are being earned.
    let mut replied = false;
    let mut out = rig.step(true, None);
    while !rig.tx.idle() {
        let offer = (!replied && rig.taken.len() >= 60).then_some(ID_VALUE);
        out = rig.step(true, offer);
        replied |= offer.is_some() && out.out_reply_busy == 0;
    }
    for _ in 0..40 * BIT_CYCLES {
        rig.step(true, None);
    }
    assert!(replied);
    assert_eq!(rig.taken.len(), 130);
    let mut d = ReplyDecoder::default();
    let replies: Vec<Reply> = rig
        .sent
        .iter()
        .filter_map(|&b| d.push(b).expect("a known tag"))
        .collect();
    let credits = replies.iter().filter(|&&r| r == Reply::Credit).count();
    assert_eq!(credits, 2, "{:x?}", rig.sent);
    assert!(replies.contains(&Reply::Data(ID_VALUE)), "{:x?}", rig.sent);
    // The reply is a tag and 8 bytes in a row.
    let tag = rig
        .sent
        .iter()
        .position(|&b| b == TAG_REPLY)
        .expect("a reply");
    assert_eq!(rig.sent[tag + 1..tag + 9], ID_VALUE.to_be_bytes());
    let _ = out;
}

/// A transaction with the FIFO full is dropped and flagged; so is a byte
/// whose stop bit is low.
#[test]
fn overflow_and_framing_are_flagged() {
    let mut rig = Rig::new();
    for _ in 0..513 {
        rig.send(Transaction::Read { addr: reg::ID });
    }
    let out = rig.drain(false);
    assert_eq!((out.out_free, out.out_overflow), (0, 1));

    let mut rig = Rig::new();
    // A start bit, eight zeros and a low stop bit.
    for _ in 0..10 * BIT_CYCLES {
        rig.step_line(0, false, None);
    }
    for _ in 0..2 * BIT_CYCLES {
        rig.step_line(1, false, None);
    }
    let out = rig.step(false, None);
    assert_eq!((out.out_framing, out.out_avail), (1, 0));
}
