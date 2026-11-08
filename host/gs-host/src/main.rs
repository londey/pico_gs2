//! `gs-host`: drives the pico-gs2 GPU over the board's USB UART
//! (`docs/design/host-interface.md`).
//!
//! It opens the serial port at 3 Mbaud, which asserts DTR# and so connects
//! the link (the GPU hands over from its built-in scene at its next frame),
//! reads `ID` and `STATUS`, streams a cube turned live over a dark blue
//! background for a while, then reads `STATUS` again and reports the
//! frames shown, the time a frame takes to draw and the link's throughput.
//! Closing the port releases DTR#, and the GPU goes back to its built-in
//! scene.
//!
//! `--screenshot <file>` instead reads the colour buffer on screen back over
//! the link (`MEM_DATA`, host-interface.md §9), saves it as a 640 × 360
//! PNG, and compares it with the built-in scene's frames as the reference
//! renderer draws them: on the board, a pixel-for-pixel check of the
//! hardware against the model.
//!
//! Usage: `gs-host [--port /dev/ttyUSB0] [--seconds 10] [--status]
//! [--screenshot file.png]` (`--status` reads `ID` and `STATUS` and streams
//! nothing).

#![deny(unsafe_code)]

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, ErrorKind, Read, Write};
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use gs_proto::{
    colour_buffer, mem_read, pixel_word, reg, unpack_words, Credits, Reply, ReplyDecoder, Status,
    Transaction, BAUD, FB_HEIGHT, FB_WIDTH, ID_VALUE,
};
use serialport::{ClearBuffer, DataBits, FlowControl, Parity, SerialPort, StopBits};
use setup_model::render::{render, Frame};
use setup_model::scene::{cube_at, cube_scene, transactions};

/// The background the host draws on, RGB565: dark blue, unlike the built-in
/// scene's grey, so it is plain which is showing.
const BACKGROUND: u16 = 0x0010;

/// Frames per turn of the live cube: one turn every 4 s at 60 frames a
/// second.
const FRAMES_PER_TURN: u32 = 240;

/// How long to wait for a read's reply.
const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

/// The command line.
struct Args {
    /// The serial port.
    port: String,

    /// How long to stream for.
    seconds: f64,

    /// Only read `ID` and `STATUS`.
    status_only: bool,

    /// Save the frame on screen here, and stream nothing.
    screenshot: Option<String>,
}

/// Parses the command line.
///
/// # Returns
///
/// The arguments.
///
/// # Errors
///
/// An unknown option or a bad value.
fn parse_args() -> Result<Args> {
    let mut args = Args {
        port: String::from("/dev/ttyUSB0"),
        seconds: 10.0,
        status_only: false,
        screenshot: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => args.port = it.next().context("--port needs a path")?,
            "--seconds" => {
                args.seconds = it
                    .next()
                    .context("--seconds needs a number")?
                    .parse()
                    .context("--seconds needs a number")?;
            }
            "--status" => args.status_only = true,
            "--screenshot" => {
                args.screenshot = Some(it.next().context("--screenshot needs a file")?);
            }
            "-h" | "--help" => {
                println!(
                    "usage: gs-host [--port /dev/ttyUSB0] [--seconds 10] [--status] [--screenshot file.png]"
                );
                std::process::exit(0);
            }
            other => bail!("unknown option {other}; try --help"),
        }
    }
    Ok(args)
}

/// The link to the GPU: the port, the host's credits, and what the GPU has
/// sent back.
struct Link {
    /// The serial port.
    port: Box<dyn SerialPort>,

    /// Transactions the GPU's FIFO has room for.
    credits: Credits,

    /// Decodes the GPU's bytes.
    decoder: ReplyDecoder,

    /// Read replies not yet collected.
    replies: VecDeque<u64>,

    /// Transactions sent.
    sent: u64,

    /// Bytes sent.
    bytes: u64,
}

impl Link {
    /// Opens the port and connects: DTR# released and asserted again, which
    /// resets the GPU's side of the link.
    ///
    /// # Arguments
    ///
    /// * `path` - The serial port.
    ///
    /// # Returns
    ///
    /// The link.
    ///
    /// # Errors
    ///
    /// The port can't be opened or set up.
    fn open(path: &str) -> Result<Link> {
        let mut port = serialport::new(path, BAUD)
            .data_bits(DataBits::Eight)
            .parity(Parity::None)
            .stop_bits(StopBits::One)
            .flow_control(FlowControl::None)
            .timeout(Duration::from_millis(5))
            .open()
            .with_context(|| format!("opening {path}"))?;
        port.write_data_terminal_ready(false)?;
        sleep(Duration::from_millis(20));
        port.clear(ClearBuffer::All)?;
        port.write_data_terminal_ready(true)?;
        sleep(Duration::from_millis(20));
        Ok(Link {
            port,
            credits: Credits::default(),
            decoder: ReplyDecoder::default(),
            replies: VecDeque::new(),
            sent: 0,
            bytes: 0,
        })
    }

    /// Takes whatever the GPU has sent: credits and replies.
    ///
    /// # Arguments
    ///
    /// * `wait` - Block (up to the port's timeout) for at least a byte.
    ///
    /// # Errors
    ///
    /// The port fails, or the GPU sent a byte that starts no message.
    fn poll(&mut self, wait: bool) -> Result<()> {
        let ready = usize::try_from(self.port.bytes_to_read()?)?;
        if ready == 0 && !wait {
            return Ok(());
        }
        let mut buf = vec![0; ready.max(1)];
        let n = match self.port.read(&mut buf) {
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::TimedOut => 0,
            Err(e) => return Err(e.into()),
        };
        for &b in &buf[..n] {
            match self.decoder.push(b) {
                Ok(Some(Reply::Credit)) => self.credits.grant(),
                Ok(Some(Reply::Data(d))) => self.replies.push_back(d),
                Ok(None) => {}
                Err(e) => bail!("the GPU sent {:#04x}, which starts no message", e.0),
            }
        }
        Ok(())
    }

    /// Sends transactions as credits allow.
    ///
    /// # Arguments
    ///
    /// * `txns` - The transactions, in order.
    ///
    /// # Errors
    ///
    /// The port fails.
    fn send(&mut self, txns: &[Transaction]) -> Result<()> {
        let mut rest = txns;
        while !rest.is_empty() {
            self.poll(self.credits.available() == 0)?;
            let mut buf = Vec::with_capacity(rest.len() * 9);
            while let Some((t, tail)) = rest.split_first() {
                if !self.credits.spend() {
                    break;
                }
                let mut b = [0; 9];
                let n = t.encode(&mut b);
                buf.extend_from_slice(&b[..n]);
                self.sent += 1;
                rest = tail;
            }
            self.port.write_all(&buf)?;
            self.bytes += u64::try_from(buf.len())?;
        }
        Ok(())
    }

    /// Reads a register.
    ///
    /// # Arguments
    ///
    /// * `addr` - Its address.
    ///
    /// # Returns
    ///
    /// Its value.
    ///
    /// # Errors
    ///
    /// The port fails, or no reply comes in time.
    fn read(&mut self, addr: u8) -> Result<u64> {
        self.send(&[Transaction::Read { addr }])?;
        self.port.flush()?;
        let deadline = Instant::now() + REPLY_TIMEOUT;
        while self.replies.is_empty() {
            if Instant::now() > deadline {
                bail!("no reply to a read of {addr:#04x}: is the board running the pico-gs2 bitstream?");
            }
            self.poll(true)?;
        }
        self.replies.pop_front().context("a reply")
    }

    /// Reads dwords of GPU memory.
    ///
    /// # Arguments
    ///
    /// * `dword` - The first dword's address.
    /// * `n` - Dwords to read.
    ///
    /// # Returns
    ///
    /// The dwords, in address order.
    ///
    /// # Errors
    ///
    /// The port fails, or the replies stop coming.
    fn read_memory(&mut self, dword: u32, n: usize) -> Result<Vec<u64>> {
        if !self.replies.is_empty() {
            bail!("replies left over from an earlier read");
        }
        let txns: Vec<Transaction> = mem_read(dword, n).collect();
        self.send(&txns)?;
        self.port.flush()?;
        let mut deadline = Instant::now() + REPLY_TIMEOUT;
        let mut had = 0;
        while self.replies.len() < n {
            if self.replies.len() != had {
                had = self.replies.len();
                deadline = Instant::now() + REPLY_TIMEOUT;
            }
            if Instant::now() > deadline {
                bail!("{} of {n} memory reads answered", self.replies.len());
            }
            self.poll(true)?;
        }
        Ok(self.replies.drain(..).collect())
    }
}

/// Reads a colour buffer back and untiles it.
///
/// # Arguments
///
/// * `link` - The link.
/// * `buffer` - The buffer, 0 to 2.
///
/// # Returns
///
/// Its pixels, RGB565, in raster order.
///
/// # Errors
///
/// The link fails.
fn read_frame(link: &mut Link, buffer: u8) -> Result<Vec<u16>> {
    let words = FB_WIDTH * FB_HEIGHT;
    let dwords = link.read_memory(colour_buffer(u32::from(buffer)), words / 4)?;
    let tiled: Vec<u16> = dwords.into_iter().flat_map(unpack_words).collect();
    Ok((0..words)
        .map(|i| tiled[pixel_word(i % FB_WIDTH, i / FB_WIDTH)])
        .collect())
}

/// An RGB565 colour as 8 bits a channel, each channel's bits repeated.
fn rgb888(c: u16) -> [u8; 3] {
    let (r, g, b) = (c >> 11, c >> 5 & 0x3F, c & 0x1F);
    [r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2].map(|v| u8::try_from(v).unwrap_or(u8::MAX))
}

/// Saves a frame as a PNG.
///
/// # Arguments
///
/// * `path` - The file.
/// * `frame` - The pixels, RGB565, in raster order.
///
/// # Errors
///
/// The file can't be written.
fn save_png(path: &str, frame: &[u16]) -> Result<()> {
    let mut encoder = png::Encoder::new(
        BufWriter::new(File::create(path).with_context(|| format!("creating {path}"))?),
        u32::try_from(FB_WIDTH)?,
        u32::try_from(FB_HEIGHT)?,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let rgb: Vec<u8> = frame.iter().flat_map(|&c| rgb888(c)).collect();
    encoder.write_header()?.write_image_data(&rgb)?;
    Ok(())
}

/// Saves the frame on screen and compares it with the built-in scene's.
///
/// # Arguments
///
/// * `link` - The link, connected.
/// * `path` - Where to save it.
///
/// # Errors
///
/// The link fails, or the file can't be written.
fn screenshot(link: &mut Link, path: &str) -> Result<()> {
    // The GPU hands over at the built-in scene's next present, and shows
    // that frame from the next frame start: wait for both, then nothing
    // changes the buffer on screen while the host presents nothing.
    sleep(Duration::from_millis(50));
    let status = Status::from_bits(link.read(reg::STATUS)?);
    println!("reading buffer {} ...", status.shown);
    let start = Instant::now();
    let frame = read_frame(link, status.shown)?;
    println!(
        "read {} pixels in {:.2} s",
        frame.len(),
        start.elapsed().as_secs_f64()
    );
    save_png(path, &frame)?;
    println!("saved {path}");
    let mut reference = Frame::new(vec![0; FB_WIDTH * FB_HEIGHT]);
    let (frames, _) = render(&mut reference, &transactions(&cube_scene()));
    let (k, differing) = frames
        .iter()
        .map(|f| f.iter().zip(&frame).filter(|(a, b)| a != b).count())
        .enumerate()
        .min_by_key(|&(_, d)| d)
        .context("the reference scene has no frames")?;
    if differing == 0 {
        println!("matches the built-in scene's frame {k} pixel for pixel");
    } else {
        println!("nearest the built-in scene's frame {k}, with {differing} pixels differing");
    }
    Ok(())
}

/// Tries to set the FTDI chip's latency timer to 1 ms (from 16), which
/// lets credits and replies come back sooner.
///
/// # Arguments
///
/// * `port` - The serial port's path.
///
/// # Returns
///
/// What happened, to report.
fn set_latency_timer(port: &str) -> String {
    let Some(name) = Path::new(port).file_name().and_then(|n| n.to_str()) else {
        return String::from("latency timer: not a usb-serial port");
    };
    let path = format!("/sys/bus/usb-serial/devices/{name}/latency_timer");
    let before = std::fs::read_to_string(&path).unwrap_or_default();
    if before.trim() == "1" {
        return String::from("latency timer: 1 ms");
    }
    match std::fs::write(&path, "1") {
        Ok(()) => format!("latency timer: {} ms -> 1 ms", before.trim()),
        Err(e) => format!(
            "latency timer: {} ms (couldn't set it to 1 ms: {e}; as root: echo 1 > {path})",
            before.trim()
        ),
    }
}

/// Prints a `STATUS` value.
fn print_status(label: &str, s: Status) {
    #[allow(clippy::cast_precision_loss)]
    let fps = if s.render_cycles > 0 {
        75e6 / f64::from(s.render_cycles)
    } else {
        0.0
    };
    println!(
        "{label}: {} frames shown, last frame drawn in {} cycles ({fps:.1} fps), {} FIFO entries free; buffer {} shown, {} drawn",
        s.frames, s.render_cycles, s.fifo_free, s.shown, s.drawn
    );
    if s.link_overflow || s.link_malformed {
        println!(
            "link errors since it connected:{}{}",
            if s.link_overflow {
                " a transaction arrived with the FIFO full;"
            } else {
                ""
            },
            if s.link_malformed {
                " a malformed byte;"
            } else {
                ""
            }
        );
    }
}

fn main() -> Result<()> {
    let args = parse_args()?;
    println!("{}", set_latency_timer(&args.port));
    let mut link = Link::open(&args.port)?;
    let id = link.read(reg::ID)?;
    if id == ID_VALUE {
        println!("ID {id:#018x} (pico-gs2, interface {})", id >> 16 & 0xFFFF);
    } else if id >> 32 == ID_VALUE >> 32 {
        println!(
            "ID {id:#018x}: pico-gs2 interface {}, but this gs-host is for interface {}; rebuild one or the other",
            id >> 16 & 0xFFFF,
            ID_VALUE >> 16 & 0xFFFF
        );
    } else {
        println!("ID {id:#018x} (not what pico-gs2 reads!)");
    }
    let before = Status::from_bits(link.read(reg::STATUS)?);
    print_status("STATUS", before);
    if let Some(path) = &args.screenshot {
        screenshot(&mut link, path)?;
        link.port.write_data_terminal_ready(false)?;
        return Ok(());
    }
    if args.status_only {
        return Ok(());
    }

    println!("streaming a live cube for {} s ...", args.seconds);
    let start = Instant::now();
    let (sent0, bytes0) = (link.sent, link.bytes);
    let mut frames: u32 = 0;
    while start.elapsed().as_secs_f64() < args.seconds {
        let angle = std::f32::consts::TAU * f32::from(u16::try_from(frames % FRAMES_PER_TURN)?)
            / f32::from(u16::try_from(FRAMES_PER_TURN)?);
        link.send(&transactions(&cube_at(angle, BACKGROUND)))?;
        frames += 1;
    }
    // Everything sent has been taken once a read comes back.
    let after = Status::from_bits(link.read(reg::STATUS)?);
    let secs = start.elapsed().as_secs_f64();
    #[allow(clippy::cast_precision_loss)]
    let (txns, bytes) = ((link.sent - sent0) as f64, (link.bytes - bytes0) as f64);
    println!(
        "sent {frames} frames in {secs:.2} s: {:.1} frames/s; {:.0} transactions/s, {:.0} bytes/s ({:.0}% of the line)",
        f64::from(frames) / secs,
        txns / secs,
        bytes / secs,
        100.0 * bytes / secs / (f64::from(BAUD) / 10.0)
    );
    print_status("STATUS", after);
    println!(
        "{} frames shown while streaming",
        after.frames.wrapping_sub(before.frames)
    );
    link.port.write_data_terminal_ready(false)?;
    Ok(())
}
