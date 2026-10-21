use crate::model::{frame, Model, MAC};
use crate::*;

fn card() -> Rtl8168<Model> {
    Rtl8168::new(Model::new()).expect("init")
}

#[test]
fn init_reads_mac_and_enables() {
    let mut c = card();
    assert_eq!(c.mac, MAC);
    assert_eq!(c.revision, 0x5480_0000);
    let m = c.hw();
    assert_eq!(m.reg8(reg::CHIPCMD), CMD_RX | CMD_TX);
    assert_eq!(m.reg32(reg::RCR), RCR_VALUE);
    assert_eq!(m.reg32(reg::TCR) & !0x7C80_0000, TCR_VALUE);
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
    for n in 0..(RING * 3 + 5) {
        let f = frame([0xFF; 6], 20 + n * 37 % 1400, n as u8);
        assert!(c.send(&f), "frame {n}");
        sent.push(f);
    }
    assert_eq!(c.sent, sent.len() as u64);
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
    for n in 0..(RING * 4) {
        let f = frame(if n % 2 == 0 { MAC } else { [0xFF; 6] }, 46 + n * 53 % 1450, n as u8);
        assert!(c.hw().deliver(&f), "frame {n} taken");
        let len = c.receive(&mut buf).expect("frame");
        assert_eq!(&buf[..len], &f[..], "frame {n} without CRC");
    }
    assert_eq!(c.received, (RING * 4) as u64);
    assert_eq!(c.receive(&mut buf), None);
}

#[test]
fn receive_filters_and_full_ring() {
    let mut c = card();
    assert!(!c.hw().deliver(&frame([2, 3, 4, 5, 6, 7], 50, 0)), "someone else's frame");
    for n in 0..RING {
        assert!(c.hw().deliver(&frame(MAC, 50, n as u8)));
    }
    assert!(!c.hw().deliver(&frame(MAC, 50, 99)), "ring full");
    assert_eq!(c.hw().missed, 1);
    let mut buf = [0u8; 2048];
    for n in 0..RING {
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
