use crate::model::{frame, Model, MAC};
use crate::*;

fn card() -> Rtl8168<Model> {
    Rtl8168::new(Model::new()).expect("init")
}

#[test]
fn init_reads_mac_and_enables() {
    let mut c = card();
    assert_eq!(c.mac, MAC);
    assert_eq!(c.revision, 0x5080_0000, "the laptop's 8168GU");
    let m = c.hw();
    assert_eq!(m.reg8(reg::CHIPCMD), CMD_RX | CMD_TX);
    assert_eq!(m.reg32(reg::RCR), RCR_VALUE);
    assert_eq!(m.reg32(reg::TCR) & !(0x7C80_0000 | TCR_TX_EMPTY), TCR_VALUE);
    assert_eq!(m.reg8(reg::CR9346), 0, "configuration locked again");
    assert_eq!(m.locked_writes, 0, "no write to a locked register");
    assert!(c.link_up());
    c.hw().set_link(false);
    assert!(!c.link_up());
}

#[test]
fn absent_card() {
    let mut m = Model::new();
    m.absent = true;
    assert_eq!(Rtl8168::new(m).err(), Some(InitError::NotAnswering));
}

#[test]
fn reset_timeout() {
    let mut m = Model::new();
    m.reset_reads = u32::MAX;
    let e = Rtl8168::new(m).err();
    assert_eq!(e, Some(InitError::ResetTimeout));
}

#[test]
fn no_memory() {
    let mut m = Model::new();
    m.page_limit = 5;
    assert_eq!(Rtl8168::new(m).err(), Some(InitError::NoMemory));
}

#[test]
fn send_frames_through_the_ring() {
    let mut c = card();
    let mut sent = Vec::new();
    for n in 0..(TX_RING * 3 + 5) {
        let f = frame([0xFF; 6], 20 + n * 37 % 1400, n as u8);
        assert!(c.send(&f), "frame {n}");
        sent.push(f);
    }
    assert_eq!(c.sent, sent.len() as u64);
    assert_eq!(c.hw().syncs, 0, "the default start-up (as os-125) drops no cache line");
    let wire = &c.hw().wire;
    assert_eq!(wire.len(), sent.len());
    for (w, f) in wire.iter().zip(&sent) {
        assert_eq!(&w[..f.len()], &f[..]);
        assert!(w.len() >= MIN_FRAME && w[f.len()..].iter().all(|&b| b == 0), "padded with zeros");
    }
}

#[test]
fn send_rejects_bad_sizes() {
    let mut c = card();
    assert!(!c.send(&[]));
    assert!(!c.send(&vec![0u8; MAX_FRAME + 1]));
    assert!(c.send(&vec![0u8; MAX_FRAME]));
    assert_eq!(c.hw().wire.len(), 1);
}

#[test]
fn send_fails_if_card_never_takes_it() {
    let mut c = card();
    c.hw().write8(reg::CHIPCMD, CMD_RX); // transmitter off: descriptors stay owned by the card
    assert!(!c.send(&frame([0xFF; 6], 50, 1)));
    assert_eq!(c.sent, 0);
}

#[test]
fn receive_frames_through_the_ring() {
    let mut c = card();
    let mut buf = [0u8; 2048];
    assert_eq!(c.receive(&mut buf), None);
    for n in 0..(RX_RING * 4) {
        let f = frame(if n % 2 == 0 { MAC } else { [0xFF; 6] }, 46 + n * 53 % 1450, n as u8);
        assert!(c.hw().deliver(&f), "frame {n} taken");
        let len = c.receive(&mut buf).expect("frame");
        assert_eq!(&buf[..len], &f[..], "frame {n} without CRC");
    }
    assert_eq!(c.received, (RX_RING * 4) as u64);
    assert_eq!(c.receive(&mut buf), None);
}

#[test]
fn receive_filters_and_full_ring() {
    let mut c = card();
    assert!(!c.hw().deliver(&frame([2, 3, 4, 5, 6, 7], 50, 0)), "someone else's frame");
    for n in 0..RX_RING {
        assert!(c.hw().deliver(&frame(MAC, 50, n as u8)));
    }
    assert!(!c.hw().deliver(&frame(MAC, 50, 99)), "ring full");
    assert_eq!(c.hw().missed, 1);
    let mut buf = [0u8; 2048];
    for n in 0..RX_RING {
        let len = c.receive(&mut buf).unwrap();
        assert_eq!(buf[14], n as u8, "in order");
        assert_eq!(len, 64);
    }
    assert!(c.hw().deliver(&frame(MAC, 50, 7)), "space again");
    assert_eq!(c.receive(&mut buf), Some(64));
}

#[test]
fn receive_skips_errors_and_cuts_to_buffer() {
    let mut c = card();
    c.hw().deliver_error();
    let f = frame(MAC, 200, 3);
    c.hw().deliver(&f);
    let mut small = [0u8; 100];
    assert_eq!(c.receive(&mut small), Some(100));
    assert_eq!(&small[..], &f[..100]);
    assert_eq!(c.dropped, 1);
    assert_eq!(c.received, 1);
}

#[test]
fn too_long_frame_is_not_taken() {
    let mut c = card();
    assert!(!c.hw().deliver(&frame(MAC, BUF, 0)));
}

#[test]
fn phy_powered_up_and_link_waited() {
    let mut m = Model::new();
    m.power_down_phy();
    let mut c = Rtl8168::new(m).unwrap();
    assert!(!c.link_up(), "PHY left powered down by the firmware");
    assert!(!c.hw().deliver(&frame(MAC, 50, 1)), "no link, nothing received");
    assert!(c.start_link(3000));
    assert_eq!(c.hw().bmcr & (1 << 11), 0, "power-down bit cleared");
    assert_ne!(c.hw().bmcr & (1 << 12), 0, "auto-negotiation on");
    assert_eq!(c.status().0, 0b1_0011);
    assert!(c.hw().deliver(&frame(MAC, 50, 1)));
    let mut buf = [0u8; 2048];
    assert_eq!(c.receive(&mut buf), Some(64));
}

#[test]
fn no_cable_gives_up_after_wait() {
    let mut m = Model::new();
    m.set_link(false);
    let mut c = Rtl8168::new(m).unwrap();
    let before = c.hw().delays_us;
    assert!(!c.start_link(500));
    assert!(c.hw().delays_us - before >= 500_000, "waited about 500 ms");
}

#[test]
fn receive_clears_overflow_and_resumes() {
    let mut c = card();
    c.hw().raise(1 << 6 | 1 << 4);
    assert!(!c.hw().deliver(&frame(MAC, 50, 2)), "stalled while the overflow bit is set");
    let mut buf = [0u8; 2048];
    assert_eq!(c.receive(&mut buf), None);
    assert_eq!(c.status().1 & ISR_ACK, 0, "status acknowledged by the poll");
    assert!(c.hw().deliver(&frame(MAC, 50, 3)), "receiving again");
    assert_eq!(c.receive(&mut buf), Some(64));
}

#[test]
fn diag_reads_back_what_was_set() {
    let mut c = card();
    let d = c.diag();
    let (rx, tx) = c.ring_addresses();
    assert_eq!((d.rdsar, d.tnpds), (rx, tx));
    assert_eq!(d.rcr, RCR_VALUE);
    assert_eq!(d.chipcmd, CMD_RX | CMD_TX);
    assert_eq!(d.rms as usize, BUF);
    assert_ne!(d.rx_opts1 & desc::OWN, 0, "first receive descriptor handed to the card");
    c.hw().raise(1 << 4);
    let mut buf = [0u8; 2048];
    c.receive(&mut buf);
    assert_eq!(c.isr_seen & (1 << 4), 1 << 4, "history kept after the bit was cleared");
    assert_eq!(c.polls, 1);
}

/// A DHCP answer (IPv4, UDP 67 -> 68) to broadcast.
fn dhcp_offer() -> Vec<u8> {
    let mut f = frame([0xFF; 6], 300, 0);
    let ip = &mut f[14..];
    ip[0] = 0x45;
    ip[9] = 17;
    ip[20..22].copy_from_slice(&67u16.to_be_bytes());
    ip[22..24].copy_from_slice(&68u16.to_be_bytes());
    f
}

#[test]
fn ring_is_one_page_with_eor_on_the_last() {
    let mut c = card();
    let mut buf = [0u8; 2048];
    // Around the ring twice: the card wraps only if the last descriptor has EOR, at start-up and after re-arming.
    for n in 0..RX_RING * 2 + 3 {
        assert!(c.hw().deliver(&frame(MAC, 50, n as u8)), "frame {n}");
        assert_eq!(c.receive(&mut buf), Some(64));
    }
    assert_eq!(c.diag().rx_owned, RX_RING, "every descriptor handed back");
    assert_eq!(c.restarts + c.resyncs, 0);
}

#[test]
fn boot_burst_fills_ring_then_receiver_restarts() {
    let mut m = Model::new();
    m.stall_after_rdu = true;
    let mut c = Rtl8168::new(m).unwrap();
    // While the driver waits for the link at start-up nobody polls: the ring fills and the card reports RDU.
    for n in 0..RX_RING + 10 {
        c.hw().deliver(&frame([0xFF; 6], 50, n as u8));
    }
    assert_eq!(c.hw().missed, 10);
    let mut buf = [0u8; 2048];
    let mut got = 0;
    while c.receive(&mut buf).is_some() {
        got += 1;
    }
    assert_eq!(got, RX_RING);
    assert!(c.rdu_count >= 1);
    // The card is stuck (what test 3 showed): new frames are lost although every descriptor is free again.
    assert!(!c.hw().deliver(&frame(MAC, 50, 1)));
    for _ in 0..STALL_POLLS {
        assert_eq!(c.receive(&mut buf), None);
    }
    assert_eq!(c.restarts, 1, "receiver restarted after the stall");
    assert_eq!(c.hw().reg8(reg::CHIPCMD), CMD_RX | CMD_TX);
    assert_eq!(c.hw().reg8(reg::CR9346), 0, "configuration locked again");
    assert_eq!(c.hw().locked_writes, 0);
    for n in 0..RX_RING * 2 {
        assert!(c.hw().deliver(&frame(MAC, 50, n as u8)), "receiving again, frame {n}");
        assert_eq!(c.receive(&mut buf), Some(64));
    }
    assert_eq!(c.received, (RX_RING * 3) as u64);
    // A quiet network afterwards: no further restarts.
    for _ in 0..STALL_POLLS * 4 {
        c.receive(&mut buf);
    }
    assert_eq!(c.restarts, 1);
}

#[test]
fn card_that_resumes_by_itself_is_not_restarted() {
    let mut c = card();
    for n in 0..RX_RING + 1 {
        c.hw().deliver(&frame(MAC, 50, n as u8));
    }
    let mut buf = [0u8; 2048];
    while c.receive(&mut buf).is_some() {}
    assert_eq!(c.receive(&mut buf), None);
    assert!(c.hw().deliver(&frame(MAC, 50, 5)), "this card goes on after RDU");
    assert_eq!(c.receive(&mut buf), Some(64));
    for _ in 0..STALL_POLLS * 2 {
        c.receive(&mut buf);
    }
    assert_eq!(c.restarts, 0);
}

#[test]
fn restart_with_kept_index_resyncs() {
    let mut m = Model::new();
    m.keep_index_on_restart = true;
    let mut c = Rtl8168::new(m).unwrap();
    let mut buf = [0u8; 2048];
    for n in 0..5 {
        c.hw().deliver(&frame(MAC, 50, n));
        c.receive(&mut buf).unwrap();
    }
    c.restart_rx(); // the driver starts again at 0, the card goes on at 5
    assert_eq!(c.hw().rx_index(), 5);
    for n in 0..(RX_RING / 2) as u8 {
        assert!(c.hw().deliver(&frame(MAC, 50, n)));
        assert_eq!(c.receive(&mut buf), Some(64), "frame {n}");
        assert_eq!(buf[14], n, "in order");
    }
    assert_eq!(c.resyncs, 1);
}

#[test]
fn first_frames_are_summarised() {
    let mut c = card();
    let mut buf = [0u8; 2048];
    let mut arp = frame([0xFF; 6], 28, 0);
    arp[12..14].copy_from_slice(&[0x08, 0x06]);
    for f in [arp.clone(), dhcp_offer(), arp] {
        c.hw().deliver(&f);
        c.receive(&mut buf).unwrap();
    }
    assert_eq!(&c.first[..3], &[(0x0806, 0), (0x0800, 68), (0x0806, 0)]);
    assert_eq!(c.first[3], (0, 0));
    assert_eq!(c.dhcp_replies, 1);
    assert_eq!(summarize(&[0; 5]), (0, 0));
}

#[test]
fn send_with_cache_option_syncs_each_frame() {
    let mut c = Rtl8168::with_options(Model::new(), Options { cache: true, ..Options::default() }).unwrap();
    for n in 0..5 {
        assert!(c.send(&frame([0xFF; 6], 100, n)));
    }
    let pages = (RX_RING / 2) as u64; // each receive buffer page is synced once at start-up
    assert_eq!(c.hw().syncs, pages + 5, "every frame written to memory before the card reads it");
}

/// 12 frames of different types, each one in its own buffer; `burst` = all arrive before the driver looks.
fn twelve_frames(no_snoop: bool, burst: bool, options: Options) -> Rtl8168<Model> {
    let mut m = Model::new();
    m.no_snoop = no_snoop;
    let mut c = Rtl8168::with_options(m, options).unwrap();
    let types: Vec<u16> = (0..12u16).map(|k| [0x0806, 0x0800, 0x86DD, 0x88CC][k as usize % 4] + (k / 4) * 0x100).collect();
    let make = |k: usize| {
        let mut f = frame([0xFF; 6], 40 + k * 13, k as u8);
        f[12..14].copy_from_slice(&types[k].to_be_bytes());
        f
    };
    let mut buf = [0u8; 2048];
    let mut check = |c: &mut Rtl8168<Model>, k: usize| {
        let f = make(k);
        assert_eq!(c.receive(&mut buf), Some(f.len()), "frame {k}");
        assert_eq!(&buf[..f.len()], &f[..], "frame {k}: its own bytes, not zeros or another frame's");
    };
    if burst {
        (0..12).for_each(|k| assert!(c.hw().deliver(&make(k))));
        (0..12).for_each(|k| check(&mut c, k));
    } else {
        for k in 0..12 {
            assert!(c.hw().deliver(&make(k)));
            check(&mut c, k);
        }
    }
    assert_eq!(c.receive(&mut buf), None);
    let want: Vec<(u16, u16)> = types[..FIRST_FRAMES].iter().map(|&t| (t, 0)).collect();
    assert_eq!(&c.first[..], &want[..], "every one of the first frames has its real type");
    assert_eq!(c.zero_type, 0);
    for (k, d) in c.detail.iter().enumerate() {
        assert_eq!(d.index as usize, k);
        assert_eq!(d.len as usize, make(k).len() + 4, "length with the CRC");
        assert_eq!(d.desc_addr, d.read_addr, "the driver reads the buffer the card was given");
        assert_eq!(d.after[..], make(k)[..16]);
    }
    c
}

#[test]
fn consecutive_frames_each_read_from_own_buffer() {
    let block = Options { rx_block: true, ..Options::default() };
    let cache = Options { cache: true, ..Options::default() };
    for options in [Options::default(), block, cache] {
        for burst in [false, true] {
            let mut c = twelve_frames(false, burst, options);
            assert_eq!(c.fixed_by_sync, 0, "coherent memory: nothing to repair");
            assert!(c.detail.iter().all(|d| d.before == d.after));
            assert_eq!(c.hw().syncs >= 12, options.cache);
        }
    }
}

#[test]
fn rx_block_buffers_are_contiguous_below_4g() {
    let mut c = Rtl8168::with_options(Model::new(), Options { rx_block: true, ..Options::default() }).unwrap();
    let first: [u64; RX_RING] = c.rx_buffer_addresses();
    for (i, a) in first.iter().enumerate() {
        assert_eq!(*a, first[0] + (i * BUF) as u64, "buffer {i}: base + i * 2048");
        assert!(*a + BUF as u64 <= 1 << 32, "below 4 GiB");
        assert!(c.hw().in_low_block(*a));
    }
    // A frame lands in the block and is read back from it.
    let f = frame(MAC, 300, 9);
    assert!(c.hw().deliver(&f));
    let mut buf = [0u8; 2048];
    assert_eq!(c.receive(&mut buf), Some(f.len()));
    assert_eq!(&buf[..f.len()], &f[..]);
    assert_eq!(c.detail[0].desc_addr, first[0]);
}

#[test]
fn full_64_bit_addresses_in_registers_and_descriptors() {
    // The model's pages lie above 4 GiB (0x1_2340_0000...): the high halves must reach the card.
    let mut c = card();
    let (rx, tx) = c.ring_addresses();
    assert!(rx > u32::MAX as u64 && tx > u32::MAX as u64);
    let d = c.diag();
    assert_eq!((d.rdsar, d.tnpds), (rx, tx), "RDSAR/TNPDS hold all 64 bits");
    let f = frame(MAC, 64, 3);
    assert!(c.hw().deliver(&f));
    let mut buf = [0u8; 2048];
    assert_eq!(c.receive(&mut buf), Some(f.len()));
    assert!(c.detail[0].desc_addr > u32::MAX as u64, "descriptor holds the high half");
    assert_eq!(c.detail[0].desc_addr, c.detail[0].read_addr);
    assert!(c.send(&frame([0xFF; 6], 64, 4)), "the card reads a transmit buffer above 4 GiB");
}

#[test]
fn rxdv_gate_left_set_stops_all_dma_unless_cleared() {
    // What test 5 looked like: link up, but 0 sent and 0 received.
    let mut m = Model::new();
    m.gate_rxdv();
    let mut c = Rtl8168::new(m).unwrap();
    assert_eq!(c.misc_change, None, "the default start-up does not touch MISC");
    assert!(!c.send(&frame([0xFF; 6], 64, 1)));
    assert!(!c.hw().deliver(&frame(MAC, 64, 2)));
    assert_eq!((c.sent, c.received), (0, 0));
    assert_ne!(c.diag().misc & MISC_RXDV_GATE, 0, "visible in log rtl");

    let mut m = Model::new();
    m.gate_rxdv();
    let mut c = Rtl8168::with_options(m, Options { rxdv: true, ..Options::default() }).unwrap();
    let (before, after) = c.misc_change.unwrap();
    assert_eq!((before & MISC_RXDV_GATE, after & MISC_RXDV_GATE), (MISC_RXDV_GATE, 0));
    assert_eq!(before & !MISC_RXDV_GATE, after, "only bit 19 changed");
    assert!(c.send(&frame([0xFF; 6], 64, 1)));
    assert!(c.hw().deliver(&frame(MAC, 64, 2)));
    let mut buf = [0u8; 2048];
    assert_eq!(c.receive(&mut buf), Some(78));

    // Gate already open: nothing is written.
    let c = Rtl8168::with_options(Model::new(), Options { rxdv: true, ..Options::default() }).unwrap();
    let (before, after) = c.misc_change.unwrap();
    assert_eq!(before, after);
}

#[test]
fn no_snoop_frames_are_read_after_dropping_the_cache() {
    // What test 4 showed: the card's data is in memory but the CPU still sees the zeros of the fresh page.
    for burst in [false, true] {
        let mut c = twelve_frames(true, burst, Options { cache: true, ..Options::default() });
        assert_eq!(c.fixed_by_sync, 12, "every frame read as zeros before the sync and right after it");
        assert!(c.detail.iter().all(|d| d.before == [0; 16]), "without the sync the driver would read zeros");
        // Each sync covers exactly the frame (plus CRC) inside its own buffer.
        assert!(c.hw().synced_bytes >= (RX_RING / 2 * 4096) as u64);
    }
}

#[test]
fn zero_type_frames_are_counted() {
    let mut c = card();
    let mut buf = [0u8; 2048];
    let mut f = frame(MAC, 50, 1);
    f[12..14].copy_from_slice(&[0, 0]);
    c.hw().deliver(&f);
    assert_eq!(c.receive(&mut buf), Some(64));
    assert_eq!(c.zero_type, 1);
    assert_eq!(c.fixed_by_sync, 0);
}

#[test]
fn default_start_takes_no_cold_steps() {
    // The default start-up stays exactly that of os-125: no C+ Command, PMCH, MAC OCP or MISC writes, no steps.
    let mut c = card();
    assert!(c.steps.as_slice().is_empty());
    let m = c.hw();
    assert_eq!(m.cplus(), 0);
    assert_eq!(m.reg8(reg::PMCH), 0);
    assert_eq!(m.ocp_value(OCP_HANDOVER), 0);
    assert_eq!(m.reg32(reg::RCR), RCR_VALUE, "no early-off bit");
}

#[test]
fn cold_card_moves_nothing_with_the_default_start() {
    // Test 6: started straight from the stick, link up, 0 sent / 0 received.
    let mut m = Model::new();
    m.cold = true;
    let mut c = Rtl8168::new(m).unwrap();
    assert!(c.link_up());
    assert!(!c.send(&frame([0xFF; 6], 64, 1)));
    assert!(!c.hw().deliver(&frame(MAC, 64, 2)));
    assert_eq!((c.sent, c.received), (0, 0));
}

#[test]
fn cold_start_steps_are_taken_and_recorded() {
    let mut m = Model::new();
    m.cold = true;
    m.gate_rxdv();
    let mut c = Rtl8168::with_options(m, Options::cold()).unwrap();
    let steps = c.steps.as_slice();
    assert!(steps.iter().all(|s| s.ok), "{:?}", steps);
    let names: std::vec::Vec<&str> = steps.iter().map(|s| s.what).collect();
    // The hand-over comes first, in this order.
    assert!(names[0].starts_with("poarta RXDV inchisa"));
    assert!(names[1].contains("coada TX"));
    assert!(names[6].starts_with("OCP E8DE bit 14"));
    assert!(names[8].starts_with("OCP E8DE bit 15"));
    // Every bit change shows the register before and after, with exactly one bit different.
    for s in steps.iter().filter(|s| s.what.contains(" bit ") && !s.what.starts_with("astept")) {
        assert!((s.before ^ s.after).count_ones() <= 1, "{:?}", s);
    }
    let m = c.hw();
    assert_eq!(m.cplus() & 0x89, 0x89, "C+ Command bits 0, 3, 7");
    assert_eq!(m.reg8(reg::PMCH) & 0x80, 0x80);
    assert_eq!(m.ocp_value(OCP_HANDOVER) & 0xC000, 0x8000, "bit 14 cleared, bit 15 set");
    assert_eq!(m.reg8(reg::MCU) & MCU_NOW_IS_OOB, 0);
    assert_eq!(m.reg32(reg::RCR), RCR_VALUE | RCR_EARLY_OFF_V2);
    assert_eq!(m.reg32(reg::MISC) & MISC_RXDV_GATE, 0, "the gate closed for the hand-over is open again");
    assert_eq!(m.reg8(reg::CHIPCMD), CMD_RX | CMD_TX);
    assert_eq!(m.locked_writes, 0);
    // The cold card now moves frames both ways.
    assert!(c.send(&frame([0xFF; 6], 64, 1)));
    assert!(c.hw().deliver(&frame(MAC, 64, 2)));
    let mut buf = [0u8; 2048];
    assert_eq!(c.receive(&mut buf), Some(78));
    let d = c.diag();
    assert_eq!((d.cplus & 0x89, d.pmch & 0x80), (0x89, 0x80), "visible in log rtl");
    // A receiver restart keeps the early-off bit.
    c.restart_rx();
    assert_eq!(c.hw().reg32(reg::RCR), RCR_VALUE | RCR_EARLY_OFF_V2);
}

#[test]
fn each_cold_variant_alone_touches_only_its_registers() {
    let c = Rtl8168::with_options(Model::new(), Options { cplus: true, ..Options::default() }).unwrap();
    assert_eq!(c.steps.len, 3);
    let mut c = Rtl8168::with_options(Model::new(), Options { phy: true, ..Options::default() }).unwrap();
    assert_eq!(c.steps.len, 3);
    assert_eq!(c.hw().cplus(), 0);
    let mut c = Rtl8168::with_options(Model::new(), Options { order: true, ..Options::default() }).unwrap();
    assert_eq!(c.steps.len, 2);
    assert_eq!(c.hw().reg32(reg::RCR), RCR_VALUE | RCR_EARLY_OFF_V2);
    let mut c = Rtl8168::with_options(Model::new(), Options { mcu: true, ..Options::default() }).unwrap();
    assert_eq!(c.steps.len, 10);
    assert_eq!(c.hw().reg32(reg::MISC) & MISC_RXDV_GATE, 0, "mcu always reopens the gate");
    assert!(c.misc_change.is_some());
}

#[test]
fn a_wait_that_runs_out_is_noted_and_start_up_goes_on() {
    let mut m = Model::new();
    m.tx_never_empty = true;
    let mut c = Rtl8168::with_options(m, Options::cold()).unwrap();
    let s = c.steps.as_slice()[1];
    assert!(s.what.contains("coada TX") && !s.ok);
    assert!(c.hw().delays_us >= 100_000, "waited about 100 ms, not forever");
    assert!(c.send(&frame([0xFF; 6], 64, 1)));
}
