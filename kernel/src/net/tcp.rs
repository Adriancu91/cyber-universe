//! TCP client (RFC 9293, retransmission timer from RFC 6298): reliable byte streams to a server.
//! `connect` opens a connection, `send` queues bytes, `recv` takes what arrived in order, `close` ends it.
//!
//! All segments go out through the TCP thread (`thread`): the network receive thread must never wait for
//! ARP itself (the ARP answer would arrive on that same thread). The TCP thread also retransmits what the
//! server has not acknowledged in time (timer with exponential back-off) and cleans up closed connections.

use super::{Ipv4, config, ipv4};
use crate::arch::x86_64::apic;
use crate::sched::{self, Mutex, WaitQueue};
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};

const FIN: u8 = 0x01;
const SYN: u8 = 0x02;
const RST: u8 = 0x04;
const PSH: u8 = 0x08;
const ACK: u8 = 0x10;

/// What we accept per segment (Ethernet 1500 - IPv4 20 - TCP 20).
const OUR_MSS: u16 = 1460;
/// Receive window: kept small enough that a full window fits in the card's 32 receive buffers.
const RCV_WINDOW: usize = 16 * 1024;
/// Most bytes `send` keeps queued (sent but unacknowledged + not yet sent).
const SEND_MAX: usize = 64 * 1024;
const RTO_INITIAL_MS: u64 = 1000;
const RTO_MIN_MS: u64 = 200;
const RTO_MAX_MS: u64 = 60_000;
/// Retransmissions of the same data before the connection is given up.
const RETRIES_MAX: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No network card or no address yet.
    NoNetwork,
    /// The server refused (RST) or reset the connection.
    Refused,
    /// No answer in time (or too many retransmissions).
    Timeout,
    /// The connection is closed (or the handle is unknown).
    Closed,
}

impl Error {
    pub fn message(self) -> &'static str {
        match self {
            Error::NoNetwork => "retea fara adresa",
            Error::Refused => "conexiune refuzata sau intrerupta de server",
            Error::Timeout => "serverul nu raspunde",
            Error::Closed => "conexiune inchisa",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    SynSent,
    Established,
    /// We sent FIN, waiting for its ACK.
    FinWait1,
    /// Our FIN is acknowledged, waiting for the server's FIN.
    FinWait2,
    /// The server sent FIN; we may still send.
    CloseWait,
    /// Both sent FIN at the same time; waiting for the ACK of ours.
    Closing,
    /// The server closed first, we sent FIN after; waiting for its ACK.
    LastAck,
    /// Both sides are done; kept a moment so late duplicates are not answered with RST.
    TimeWait,
    Closed,
}

/// One connection (transmission control block).
struct Tcb {
    id: u32,
    state: State,
    local_port: u16,
    remote: Ipv4,
    remote_port: u16,
    iss: u32,
    /// Oldest unacknowledged sequence number, next one to send.
    snd_una: u32,
    snd_nxt: u32,
    /// Bytes from `snd_una` on: sent-but-unacknowledged first, then not yet sent.
    sendbuf: VecDeque<u8>,
    /// The server's window and largest segment.
    snd_wnd: u32,
    mss: usize,
    /// `close` was called / our FIN went out (its sequence number is `fin_seq`).
    fin_queued: bool,
    fin_sent: bool,
    fin_seq: u32,
    /// Next sequence number expected from the server; data received in order; segments that came early.
    rcv_nxt: u32,
    recvbuf: VecDeque<u8>,
    early: Vec<(u32, Vec<u8>)>,
    peer_fin: bool,
    reset: bool,
    /// Retransmission timer: current timeout, deadline tick (0 = off), retries of the oldest data.
    rto_ms: u64,
    rto_at: u64,
    retries: u32,
    /// Round-trip estimate (RFC 6298), in ms; 0 = no sample yet. One segment is timed at a time.
    srtt: u64,
    rttvar: u64,
    timed: Option<(u32, u64)>,
    /// The program let go of the connection (`close` returned); cleaned up when finished.
    orphan: bool,
    /// Tick of the last state change (for TIME-WAIT and orphan clean-up).
    since: u64,
    retransmits: u32,
}

/// A segment waiting to go out: (destination, bytes).
static OUTBOX: Mutex<VecDeque<(Ipv4, Vec<u8>)>> = Mutex::new(VecDeque::new());
static CONNS: Mutex<Vec<Tcb>> = Mutex::new(Vec::new());
/// Wakes the TCP thread (new segments to send) / the programs waiting on a connection.
static WORK: WaitQueue = WaitQueue::new();
static CHANGED: WaitQueue = WaitQueue::new();
static NEXT_ID: AtomicU32 = AtomicU32::new(1);
static NEXT_PORT: AtomicU16 = AtomicU16::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);

/// Test hooks: the next data segment we send / the n-th data segment we receive is "lost".
#[cfg(feature = "qemu-test")]
pub static DROP_NEXT_SEND: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "qemu-test")]
pub static DROP_RECV_AFTER: AtomicU32 = AtomicU32::new(0);

/// Sequence-number arithmetic modulo 2^32: is `a` before `b`?
fn before(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

fn ms_to_ticks(ms: u64) -> u64 {
    (ms * apic::HZ).div_ceil(1000)
}

/// Starts the TCP thread (once).
fn start() {
    if !STARTED.swap(true, Ordering::AcqRel) {
        sched::spawn_system(thread);
    }
}

/// Builds a segment with a correct checksum.
#[allow(clippy::too_many_arguments)]
fn segment(src: Ipv4, dst: Ipv4, sport: u16, dport: u16, seq: u32, ack: u32, flags: u8, window: u16, data: &[u8]) -> Vec<u8> {
    let options: &[u8] = if flags & SYN != 0 { &[2, 4, (OUR_MSS >> 8) as u8, OUR_MSS as u8] } else { &[] };
    let header = 20 + options.len();
    let mut s = Vec::with_capacity(header + data.len());
    s.extend_from_slice(&sport.to_be_bytes());
    s.extend_from_slice(&dport.to_be_bytes());
    s.extend_from_slice(&seq.to_be_bytes());
    s.extend_from_slice(&ack.to_be_bytes());
    s.push(((header / 4) as u8) << 4);
    s.push(flags);
    s.extend_from_slice(&window.to_be_bytes());
    s.extend_from_slice(&[0, 0, 0, 0]); // checksum, urgent pointer
    s.extend_from_slice(options);
    s.extend_from_slice(data);
    let sum = ipv4::pseudo_checksum(src, dst, ipv4::PROTO_TCP, &s);
    s[16..18].copy_from_slice(&sum.to_be_bytes());
    s
}

impl Tcb {
    fn window(&self) -> u16 {
        RCV_WINDOW.saturating_sub(self.recvbuf.len()).min(u16::MAX as usize) as u16
    }

    /// Queues a segment with our current acknowledgement and window.
    fn emit(&self, out: &mut Vec<(Ipv4, Vec<u8>)>, seq: u32, flags: u8, data: &[u8]) {
        let Some(cfg) = config() else { return };
        let ack = if flags & SYN != 0 && self.state == State::SynSent { 0 } else { self.rcv_nxt };
        let flags = if self.state == State::SynSent { flags } else { flags | ACK };
        out.push((self.remote, segment(cfg.ip, self.remote, self.local_port, self.remote_port, seq, ack, flags, self.window(), data)));
    }

    fn set_state(&mut self, s: State) {
        self.state = s;
        self.since = apic::ticks();
    }

    /// Bytes of `sendbuf` already sent (the rest waits for the window).
    fn sent_data(&self) -> usize {
        let in_flight = self.snd_nxt.wrapping_sub(self.snd_una) as usize;
        let fin = (self.fin_sent && !before(self.fin_seq, self.snd_una)) as usize;
        in_flight - fin
    }

    fn arm_timer(&mut self) {
        if self.rto_at == 0 {
            self.rto_at = apic::ticks() + ms_to_ticks(self.rto_ms);
        }
    }

    /// Sends what the server's window allows, then our FIN if `close` asked for it and all data went out.
    fn pump(&mut self, out: &mut Vec<(Ipv4, Vec<u8>)>) {
        if !matches!(self.state, State::Established | State::CloseWait) {
            return;
        }
        loop {
            let sent = self.sent_data();
            let unsent = self.sendbuf.len() - sent;
            let in_flight = self.snd_nxt.wrapping_sub(self.snd_una) as usize;
            let room = (self.snd_wnd as usize).saturating_sub(in_flight);
            let n = unsent.min(self.mss).min(room);
            if n == 0 {
                break;
            }
            let data: Vec<u8> = self.sendbuf.range(sent..sent + n).copied().collect();
            let seq = self.snd_nxt;
            self.snd_nxt = self.snd_nxt.wrapping_add(n as u32);
            if self.timed.is_none() {
                self.timed = Some((self.snd_nxt, apic::ticks()));
            }
            self.arm_timer();
            #[cfg(feature = "qemu-test")]
            if DROP_NEXT_SEND.swap(false, Ordering::Relaxed) {
                crate::info!("tcp: test: segmentul {} \"pierdut\" la trimitere", seq);
                continue;
            }
            self.emit(out, seq, PSH, &data);
        }
        if self.fin_queued && !self.fin_sent && self.sent_data() == self.sendbuf.len() {
            self.fin_sent = true;
            self.fin_seq = self.snd_nxt;
            self.snd_nxt = self.snd_nxt.wrapping_add(1);
            self.emit(out, self.fin_seq, FIN, &[]);
            self.arm_timer();
            let next = if self.state == State::CloseWait { State::LastAck } else { State::FinWait1 };
            self.set_state(next);
        }
    }

    /// The retransmission timer ran out: resend the oldest unacknowledged segment, double the timeout.
    fn retransmit(&mut self, out: &mut Vec<(Ipv4, Vec<u8>)>) {
        self.retries += 1;
        self.retransmits += 1;
        if self.retries > RETRIES_MAX {
            crate::warn!("tcp: {}:{} nu mai raspunde, renunt", fmt(self.remote), self.remote_port);
            self.reset = true;
            self.set_state(State::Closed);
            return;
        }
        self.timed = None; // Karn: no round-trip sample from a retransmitted segment
        self.rto_ms = (self.rto_ms * 2).min(RTO_MAX_MS);
        self.rto_at = apic::ticks() + ms_to_ticks(self.rto_ms);
        if self.state == State::SynSent {
            self.emit(out, self.iss, SYN, &[]);
            return;
        }
        let n = self.sent_data().min(self.mss);
        let data: Vec<u8> = self.sendbuf.range(..n).copied().collect();
        let fin_here = self.fin_sent && n == self.sendbuf.len() && !before(self.fin_seq, self.snd_una);
        if n > 0 || fin_here {
            self.emit(out, self.snd_una, if fin_here { FIN | PSH } else { PSH }, &data);
        }
    }

    /// An acceptable ACK: forget acknowledged bytes, update the round-trip estimate and the timer.
    fn acked(&mut self, ack: u32) {
        let mut n = ack.wrapping_sub(self.snd_una) as usize;
        if self.fin_sent && !before(ack, self.fin_seq.wrapping_add(1)) {
            n -= 1; // our FIN is acknowledged
        }
        self.sendbuf.drain(..n.min(self.sendbuf.len()));
        self.snd_una = ack;
        if let Some((seq, at)) = self.timed {
            if !before(ack, seq) {
                let rtt = (apic::ticks() - at) * 1000 / apic::HZ;
                if self.srtt == 0 {
                    self.srtt = rtt;
                    self.rttvar = rtt / 2;
                } else {
                    self.rttvar = (3 * self.rttvar + self.srtt.abs_diff(rtt)) / 4;
                    self.srtt = (7 * self.srtt + rtt) / 8;
                }
                self.timed = None;
            }
        }
        self.retries = 0;
        let base = if self.srtt == 0 { RTO_INITIAL_MS } else { self.srtt + (4 * self.rttvar).max(10) };
        self.rto_ms = base.clamp(RTO_MIN_MS, RTO_MAX_MS);
        self.rto_at = 0;
        if self.snd_una != self.snd_nxt {
            self.arm_timer();
        }
    }

    /// Takes in-order data (and early segments that now fit).
    fn take_data(&mut self, seq: u32, data: &[u8]) {
        // Trim what we already have.
        let skip = self.rcv_nxt.wrapping_sub(seq) as usize;
        if !before(self.rcv_nxt, seq) && skip < data.len() {
            let room = RCV_WINDOW.saturating_sub(self.recvbuf.len());
            let part = &data[skip..data.len().min(skip + room)];
            self.recvbuf.extend(part);
            self.rcv_nxt = self.rcv_nxt.wrapping_add(part.len() as u32);
            // Early segments that are now next in line.
            while let Some(i) = self.early.iter().position(|(s, d)| {
                !before(self.rcv_nxt, *s) && before(self.rcv_nxt, s.wrapping_add(d.len() as u32))
            }) {
                let (s, d) = self.early.swap_remove(i);
                let from = self.rcv_nxt.wrapping_sub(s) as usize;
                self.recvbuf.extend(&d[from..]);
                self.rcv_nxt = self.rcv_nxt.wrapping_add((d.len() - from) as u32);
            }
            let rcv_nxt = self.rcv_nxt;
            self.early.retain(|(s, d)| before(rcv_nxt, s.wrapping_add(d.len() as u32)));
        } else if before(self.rcv_nxt, seq)
            && (seq.wrapping_sub(self.rcv_nxt) as usize) < RCV_WINDOW
            && self.early.len() < 64
            && !self.early.iter().any(|(s, _)| *s == seq)
        {
            self.early.push((seq, data.to_vec()));
        }
    }
}

fn fmt(ip: Ipv4) -> alloc::string::String {
    alloc::format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

/// Hands segments to the TCP thread.
fn transmit(out: Vec<(Ipv4, Vec<u8>)>) {
    if out.is_empty() {
        return;
    }
    OUTBOX.lock().extend(out);
    WORK.wake_all();
}

/// Answers a segment that belongs to no connection with RST (unless it is one).
fn refuse(src: Ipv4, dst: Ipv4, sport: u16, dport: u16, seq: u32, ack: u32, flags: u8, len: u32) {
    if flags & RST != 0 {
        return;
    }
    let s = if flags & ACK != 0 {
        segment(dst, src, dport, sport, ack, 0, RST, 0, &[])
    } else {
        let end = seq.wrapping_add(len).wrapping_add((flags & SYN != 0) as u32 + (flags & FIN != 0) as u32);
        segment(dst, src, dport, sport, 0, end, RST | ACK, 0, &[])
    };
    transmit(alloc::vec![(src, s)]);
}

/// A received TCP segment (inside an IPv4 packet from `src` to `dst`). Runs on the network receive thread.
pub fn handle(src: Ipv4, dst: Ipv4, s: &[u8]) {
    if s.len() < 20 || ipv4::pseudo_checksum(src, dst, ipv4::PROTO_TCP, s) != 0 {
        return;
    }
    let sport = u16::from_be_bytes([s[0], s[1]]);
    let dport = u16::from_be_bytes([s[2], s[3]]);
    let seq = u32::from_be_bytes([s[4], s[5], s[6], s[7]]);
    let ack = u32::from_be_bytes([s[8], s[9], s[10], s[11]]);
    let offset = ((s[12] >> 4) as usize) * 4;
    let flags = s[13];
    let window = u16::from_be_bytes([s[14], s[15]]) as u32;
    if offset < 20 || offset > s.len() {
        return;
    }
    let options = &s[20..offset];
    let data = &s[offset..];

    #[cfg(feature = "qemu-test")]
    if !data.is_empty() {
        let n = DROP_RECV_AFTER.load(Ordering::Relaxed);
        if n > 0 && DROP_RECV_AFTER.fetch_sub(1, Ordering::Relaxed) == 1 {
            crate::info!("tcp: test: segmentul primit {} \"pierdut\"", seq);
            return;
        }
    }

    let mut out = Vec::new();
    let mut conns = CONNS.lock();
    let Some(c) = conns.iter_mut().find(|c| c.remote == src && c.remote_port == sport && c.local_port == dport) else {
        drop(conns);
        refuse(src, dst, sport, dport, seq, ack, flags, data.len() as u32);
        return;
    };
    if c.state == State::Closed {
        return;
    }

    if c.state == State::SynSent {
        if flags & ACK != 0 && ack != c.iss.wrapping_add(1) {
            if flags & RST == 0 {
                drop(conns);
                refuse(src, dst, sport, dport, seq, ack, flags, data.len() as u32);
            }
            return;
        }
        if flags & RST != 0 {
            if flags & ACK != 0 {
                c.reset = true;
                c.set_state(State::Closed);
                drop(conns);
                CHANGED.wake_all();
            }
            return;
        }
        if flags & SYN != 0 && flags & ACK != 0 {
            c.mss = parse_mss(options).unwrap_or(536).min(OUR_MSS as usize);
            c.rcv_nxt = seq.wrapping_add(1);
            c.snd_wnd = window;
            c.acked(ack);
            c.set_state(State::Established);
            c.emit(&mut out, c.snd_nxt, ACK, &[]);
            c.pump(&mut out);
        }
        drop(conns);
        transmit(out);
        CHANGED.wake_all();
        return;
    }

    // Is the segment inside our receive window? (RFC 9293 3.10.7.4, simplified.)
    let wnd = RCV_WINDOW.saturating_sub(c.recvbuf.len()).max(1) as u32;
    let len = data.len() as u32 + (flags & FIN != 0) as u32;
    let acceptable = if len == 0 {
        !before(seq, c.rcv_nxt) && before(seq, c.rcv_nxt.wrapping_add(wnd.max(1))) || seq == c.rcv_nxt
    } else {
        before(seq, c.rcv_nxt.wrapping_add(wnd)) && before(c.rcv_nxt, seq.wrapping_add(len))
    };
    if !acceptable {
        if flags & RST == 0 {
            c.emit(&mut out, c.snd_nxt, ACK, &[]); // tell the server where we are
        }
        drop(conns);
        transmit(out);
        return;
    }
    if flags & RST != 0 {
        c.reset = true;
        c.set_state(State::Closed);
        drop(conns);
        CHANGED.wake_all();
        return;
    }
    if flags & ACK == 0 {
        return;
    }
    // Acknowledgement of what we sent.
    if before(c.snd_una, ack) && !before(c.snd_nxt, ack) {
        c.acked(ack);
        if c.fin_sent && !before(ack, c.fin_seq.wrapping_add(1)) {
            match c.state {
                State::FinWait1 => c.set_state(State::FinWait2),
                State::Closing => c.set_state(State::TimeWait),
                State::LastAck => c.set_state(State::Closed),
                _ => {}
            }
        }
    }
    if !before(ack, c.snd_una) {
        c.snd_wnd = window;
    }
    // Data and FIN.
    let mut answer = false;
    if !data.is_empty() && matches!(c.state, State::Established | State::FinWait1 | State::FinWait2) {
        c.take_data(seq, data);
        answer = true;
    }
    if flags & FIN != 0 && !c.peer_fin && seq.wrapping_add(data.len() as u32) == c.rcv_nxt {
        c.peer_fin = true;
        c.rcv_nxt = c.rcv_nxt.wrapping_add(1);
        answer = true;
        match c.state {
            State::Established => c.set_state(State::CloseWait),
            State::FinWait1 => c.set_state(State::Closing),
            State::FinWait2 => c.set_state(State::TimeWait),
            _ => {}
        }
    } else if flags & FIN != 0 {
        answer = true; // FIN before missing data: acknowledge what we have
    }
    if answer {
        c.emit(&mut out, c.snd_nxt, ACK, &[]);
    }
    c.pump(&mut out);
    drop(conns);
    transmit(out);
    CHANGED.wake_all();
}

fn parse_mss(options: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < options.len() {
        match options[i] {
            0 => break,
            1 => i += 1,
            kind => {
                let len = *options.get(i + 1)? as usize;
                if len < 2 {
                    return None;
                }
                if kind == 2 && len == 4 {
                    return Some(u16::from_be_bytes([*options.get(i + 2)?, *options.get(i + 3)?]) as usize);
                }
                i += len;
            }
        }
    }
    None
}

/// The TCP thread: sends queued segments, runs the retransmission timers, cleans up.
fn thread() {
    loop {
        WORK.wait_until_or(apic::ticks() + ms_to_ticks(50), || OUTBOX.try_lock().is_some_and(|o| !o.is_empty()));
        loop {
            let next = OUTBOX.lock().pop_front();
            let Some((dst, seg)) = next else { break };
            ipv4::send(dst, ipv4::PROTO_TCP, &seg);
        }
        let now = apic::ticks();
        let mut out = Vec::new();
        let mut changed = false;
        let mut conns = CONNS.lock();
        for c in conns.iter_mut() {
            if c.rto_at != 0 && now >= c.rto_at && c.state != State::Closed && c.state != State::TimeWait {
                c.retransmit(&mut out);
                changed |= c.state == State::Closed;
            }
            if c.state == State::TimeWait && now - c.since > 2 * apic::HZ {
                c.set_state(State::Closed);
                changed = true;
            }
            if c.orphan && c.state == State::FinWait2 && now - c.since > 30 * apic::HZ {
                c.set_state(State::Closed);
            }
        }
        conns.retain(|c| !(c.orphan && c.state == State::Closed));
        drop(conns);
        transmit(out);
        if changed {
            CHANGED.wake_all();
        }
    }
}

/// Opens a connection to `ip`:`port`, waiting up to `timeout_ms` for the server. Returns its handle.
pub fn connect(ip: Ipv4, port: u16, timeout_ms: u64) -> Result<u32, Error> {
    let cfg = config().filter(|c| c.ip != [0; 4]).ok_or(Error::NoNetwork)?;
    start();
    if NEXT_PORT.load(Ordering::Relaxed) == 0 {
        NEXT_PORT.store((apic::ticks() % 8192) as u16 + 1, Ordering::Relaxed);
    }
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let iss = (apic::ticks() as u32).wrapping_mul(0x9E37_79B9) ^ u32::from_be_bytes([cfg.mac[2], cfg.mac[3], cfg.mac[4], cfg.mac[5]]) ^ id;
    let mut conns = CONNS.lock();
    let local_port = loop {
        let p = 49152 + NEXT_PORT.fetch_add(1, Ordering::Relaxed) % 16384;
        if !conns.iter().any(|c| c.local_port == p) {
            break p;
        }
    };
    let mut c = Tcb {
        id,
        state: State::SynSent,
        local_port,
        remote: ip,
        remote_port: port,
        iss,
        snd_una: iss,
        snd_nxt: iss.wrapping_add(1),
        sendbuf: VecDeque::new(),
        snd_wnd: 0,
        mss: 536,
        fin_queued: false,
        fin_sent: false,
        fin_seq: 0,
        rcv_nxt: 0,
        recvbuf: VecDeque::new(),
        early: Vec::new(),
        peer_fin: false,
        reset: false,
        rto_ms: RTO_INITIAL_MS,
        rto_at: 0,
        retries: 0,
        srtt: 0,
        rttvar: 0,
        timed: None,
        orphan: false,
        since: apic::ticks(),
        retransmits: 0,
    };
    let mut out = Vec::new();
    c.emit(&mut out, iss, SYN, &[]);
    c.arm_timer();
    conns.push(c);
    drop(conns);
    transmit(out);
    let deadline = apic::ticks() + ms_to_ticks(timeout_ms);
    let mut result = Err(Error::Timeout);
    CHANGED.wait_until_or(deadline, || {
        let Some(conns) = CONNS.try_lock() else { return false };
        match conns.iter().find(|c| c.id == id) {
            Some(c) if c.state == State::SynSent => false,
            Some(c) if c.reset || c.state == State::Closed => {
                result = Err(if c.reset && c.retransmits <= RETRIES_MAX { Error::Refused } else { Error::Timeout });
                true
            }
            Some(_) => {
                result = Ok(id);
                true
            }
            None => true,
        }
    });
    if result.is_err() {
        CONNS.lock().retain(|c| c.id != id);
    }
    result
}

/// Queues `data` for sending (waits while more than 64 KiB is still unacknowledged).
pub fn send(id: u32, mut data: &[u8], timeout_ms: u64) -> Result<(), Error> {
    let deadline = apic::ticks() + ms_to_ticks(timeout_ms);
    while !data.is_empty() {
        let mut out = Vec::new();
        let mut queued = false;
        {
            let mut conns = CONNS.lock();
            let c = conns.iter_mut().find(|c| c.id == id).ok_or(Error::Closed)?;
            if c.reset {
                return Err(Error::Refused);
            }
            if c.fin_queued || !matches!(c.state, State::Established | State::CloseWait) {
                return Err(Error::Closed);
            }
            let room = SEND_MAX.saturating_sub(c.sendbuf.len());
            if room > 0 {
                let n = room.min(data.len());
                c.sendbuf.extend(&data[..n]);
                data = &data[n..];
                c.pump(&mut out);
                queued = true;
            }
        }
        transmit(out);
        if !queued {
            let full = CHANGED.wait_until_or(deadline, || {
                CONNS.try_lock().is_some_and(|c| c.iter().find(|c| c.id == id).is_none_or(|c| c.sendbuf.len() < SEND_MAX || c.reset))
            });
            if !full {
                return Err(Error::Timeout);
            }
        }
    }
    Ok(())
}

/// Takes up to `max` received bytes, waiting up to `timeout_ms` for some. An empty result means the
/// server closed its side (end of stream).
pub fn recv(id: u32, max: usize, timeout_ms: u64) -> Result<Vec<u8>, Error> {
    let deadline = apic::ticks() + ms_to_ticks(timeout_ms);
    let mut result = Err(Error::Timeout);
    let mut out = Vec::new();
    CHANGED.wait_until_or(deadline, || {
        let Some(mut conns) = CONNS.try_lock() else { return false };
        let Some(c) = conns.iter_mut().find(|c| c.id == id) else {
            result = Err(Error::Closed);
            return true;
        };
        if !c.recvbuf.is_empty() {
            let was_full = c.window() < c.mss as u16;
            let n = max.min(c.recvbuf.len());
            result = Ok(c.recvbuf.drain(..n).collect());
            if was_full {
                c.emit(&mut out, c.snd_nxt, ACK, &[]); // window update: the server may send again
            }
            return true;
        }
        if c.reset {
            result = Err(Error::Refused);
            return true;
        }
        if c.peer_fin || c.state == State::Closed {
            result = Ok(Vec::new());
            return true;
        }
        false
    });
    transmit(out);
    result
}

/// Closes our side (after the queued data) and lets go of the connection. Waits up to `timeout_ms`
/// for the server to acknowledge; the rest of the closing happens in the background.
pub fn close(id: u32, timeout_ms: u64) {
    let mut out = Vec::new();
    {
        let mut conns = CONNS.lock();
        let Some(c) = conns.iter_mut().find(|c| c.id == id) else { return };
        c.fin_queued = true;
        c.pump(&mut out);
    }
    transmit(out);
    let deadline = apic::ticks() + ms_to_ticks(timeout_ms);
    CHANGED.wait_until_or(deadline, || {
        CONNS.try_lock().is_some_and(|c| {
            c.iter().find(|c| c.id == id).is_none_or(|c| {
                c.reset || matches!(c.state, State::FinWait2 | State::TimeWait | State::Closed)
            })
        })
    });
    let mut conns = CONNS.lock();
    if let Some(c) = conns.iter_mut().find(|c| c.id == id) {
        c.orphan = true;
        if c.reset || matches!(c.state, State::SynSent | State::Established | State::CloseWait) {
            c.set_state(State::Closed);
        }
    }
    conns.retain(|c| !(c.orphan && c.state == State::Closed));
}

/// Retransmissions made on connection `id` so far.
#[cfg(feature = "qemu-test")]
pub fn retransmissions(id: u32) -> u32 {
    CONNS.lock().iter().find(|c| c.id == id).map_or(0, |c| c.retransmits)
}
