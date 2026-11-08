//! A host on the serial link, as `gs-host` behaves: it sends transactions
//! while it has credits, one transaction's bytes at a time, and decodes the
//! credit bytes and read replies the GPU sends back
//! (`docs/design/host-interface.md` §5).

use std::collections::VecDeque;

use gs_proto::{Credits, Reply, ReplyDecoder, Transaction};

use crate::uart::{UartRx, UartTx};

/// The host's side of the link.
#[derive(Clone, Debug, Default)]
pub struct Host {
    /// Transactions not yet sent.
    pending: VecDeque<Transaction>,

    /// Transactions the FIFO has room for, as the host counts them.
    credits: Credits,

    /// Decodes what the GPU sends.
    decoder: ReplyDecoder,

    /// The line to the GPU.
    tx: UartTx,

    /// The line from the GPU.
    rx: UartRx,

    /// Read replies, in order.
    pub replies: Vec<u64>,

    /// Credit bytes received.
    pub credit_bytes: usize,

    /// Transactions sent.
    pub sent: usize,

    /// Bytes from the GPU that started no message.
    pub unknown_bytes: usize,
}

impl Host {
    /// Queues transactions to send.
    ///
    /// # Arguments
    ///
    /// * `txns` - The transactions, in order.
    pub fn send(&mut self, txns: impl IntoIterator<Item = Transaction>) {
        self.pending.extend(txns);
    }

    /// Whether everything queued has gone out on the line.
    ///
    /// # Returns
    ///
    /// `true` when all sent.
    pub fn done(&self) -> bool {
        self.pending.is_empty() && self.tx.idle()
    }

    /// The host's credits now.
    ///
    /// # Returns
    ///
    /// Transactions it may still send.
    pub fn credits(&self) -> u32 {
        self.credits.available()
    }

    /// A new connection (DTR asserted again): the GPU's FIFO is empty.
    pub fn reconnect(&mut self) {
        self.credits = Credits::default();
        self.decoder = ReplyDecoder::default();
    }

    /// One core cycle.
    ///
    /// # Arguments
    ///
    /// * `gpu_tx` - The GPU's transmit line this cycle.
    ///
    /// # Returns
    ///
    /// The host's transmit line, the GPU's receive line, this cycle.
    pub fn clock(&mut self, gpu_tx: u8) -> u8 {
        if let Some(b) = self.rx.clock(gpu_tx) {
            match self.decoder.push(b) {
                Ok(Some(Reply::Credit)) => {
                    self.credit_bytes += 1;
                    self.credits.grant();
                }
                Ok(Some(Reply::Data(d))) => self.replies.push(d),
                Ok(None) => {}
                Err(_) => self.unknown_bytes += 1,
            }
        }
        if self.tx.idle() && !self.pending.is_empty() && self.credits.spend() {
            if let Some(t) = self.pending.pop_front() {
                let mut bytes = [0; 9];
                let n = t.encode(&mut bytes);
                for &b in &bytes[..n] {
                    self.tx.push(b);
                }
                self.sent += 1;
            }
        }
        self.tx.line()
    }
}
