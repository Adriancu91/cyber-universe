//! Host tests of the driver logic against the software model.

use crate::model::Model;
use crate::*;
use std::vec::Vec;

fn start(m: Model, o: Options) -> E1000e<Model> {
    let f = m.family;
    match E1000e::init(m, f, o) {
        Ok(c) => c,
        Err(e) => panic!("init: {:?}", e),
    }
}

#[test]
fn identifies_the_workstation_card() {
    assert_eq!(identify(0x8086, 0x15E3), Some((Family::Pch, "I219-LM")));
    assert_eq!(identify(0x8086, 0x10D3), Some((Family::E82574, "82574L (e1000e)")));
    assert_eq!(identify(0x10EC, 0x15E3), None);
    assert_eq!(identify(0x8086, 0x1234), None);
    // every id appears once
    for (i, (a, _, _)) in CARDS.iter().enumerate() {
        assert!(CARDS[i + 1..].iter().all(|(b, _, _)| b != a), "id dublat {:#x}", a);
    }
}

#[test]
fn starts_82574_and_reads_mac_and_phy() {
    let c = start(Model::new(Family::E82574), Options::DEFAULT);
    assert_eq!(c.mac, [0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
    assert_eq!(c.phy_id, 0x0141_0CB0);
    assert_eq!(c.hw.resets, 1);
    assert!(c.hw.violations.is_empty(), "{:?}", c.hw.violations);
}

#[test]
fn pch_uses_semaphore_and_phy_address_2() {
    let mut c = start(Model::new(Family::Pch), Options::DEFAULT);
    assert_eq!(c.phy_id, 0x0154_00A0, "PHY-ul I219 e la adresa 2");
    assert!(c.hw.violations.is_empty(), "{:?}", c.hw.violations);
    assert_eq!(c.hw.lanphypc_cycles, 0, "PHY-ul raspunde: fara ciclu LANPHYPC");
    assert_eq!(c.hw.read32(reg::EXTCNF_CTRL) & EXTCNF_SWFLAG, 0, "semaforul eliberat");
    assert_eq!(c.phy_status(), Some(0x796D));
    assert!(c.hw.violations.is_empty());
    assert!(c.steps[..c.step_count].iter().any(|s| s.what.starts_with("NVM incarcat")));
}

#[test]
fn pch_wakes_a_sleeping_phy_with_lanphypc() {
    let mut m = Model::new(Family::Pch);
    m.phy_asleep = true;
    let c = start(m, Options::DEFAULT);
    assert_eq!(c.hw.lanphypc_cycles, 1);
    assert_eq!(c.phy_id, 0x0154_00A0);
    let steps = &c.steps[..c.step_count];
    assert!(steps.iter().any(|s| s.what == "PHY raspunde? (ID1)" && s.after == 0xFFFF));
    assert!(steps.iter().any(|s| s.what == "PHY raspunde dupa LANPHYPC?" && s.after == 0x0154));
}

#[test]
fn pch_without_lanphypc_option_leaves_phy_asleep() {
    let mut m = Model::new(Family::Pch);
    m.phy_asleep = true;
    let o = Options { lanphypc: false, ..Options::DEFAULT };
    let c = start(m, o);
    assert_eq!(c.hw.lanphypc_cycles, 0);
    assert_eq!(c.phy_id, 0, "PHY-ul nu raspunde, iar driverul o spune");
}

#[test]
fn pch_semaphore_held_by_firmware_is_an_error() {
    let mut m = Model::new(Family::Pch);
    m.fw_holds_semaphore = true;
    let r = E1000e::init(m, Family::Pch, Options::DEFAULT);
    assert_eq!(r.err(), Some(InitError::SemaphoreBusy));
}

#[test]
fn stops_rings_left_running_before_reset() {
    let mut m = Model::new(Family::Pch);
    m.rings_running = true;
    m.regs.insert(reg::RCTL, RCTL_EN);
    let c = start(m, Options::DEFAULT);
    assert!(c.hw.violations.is_empty(), "{:?}", c.hw.violations);
    let mut m = Model::new(Family::Pch);
    m.regs.insert(reg::RCTL, RCTL_EN);
    let o = Options { stop_before_reset: false, ..Options::DEFAULT };
    let c = start(m, o);
    assert_eq!(c.hw.violations, ["reset cu RX/TX pornite"], "fara optiune, modelul vede resetul cu RX pornit");
}

#[test]
fn errors_are_reported() {
    let mut m = Model::new(Family::E82574);
    m.all_ones = true;
    assert_eq!(E1000e::init(m, Family::E82574, Options::DEFAULT).err(), Some(InitError::NotAnswering));
    let mut m = Model::new(Family::E82574);
    m.reset_stuck = true;
    assert_eq!(E1000e::init(m, Family::E82574, Options::DEFAULT).err(), Some(InitError::ResetStuck));
    let mut m = Model::new(Family::E82574);
    m.nvm_mac = None;
    m.regs.clear();
    assert_eq!(E1000e::init(m, Family::E82574, Options::DEFAULT).err(), Some(InitError::NoMac));
}

#[test]
fn missing_lan_init_done_is_logged_not_fatal() {
    let mut m = Model::new(Family::Pch);
    m.no_lan_init_done = true;
    let c = start(m, Options::DEFAULT);
    assert!(c.steps[..c.step_count].iter().any(|s| s.what == "NVM: LAN_INIT_DONE nu a venit"));
}

#[test]
fn rings_are_in_one_block_with_64_bit_addresses() {
    let mut c = start(Model::new(Family::Pch), Options::DEFAULT);
    let b = c.block_phys();
    assert_eq!(c.hw.read32(reg::RDBAL) as u64, b);
    assert_eq!(c.hw.read32(reg::RDBAH), 0);
    assert_eq!(c.hw.read32(reg::TDBAL) as u64, b + 4096);
    assert_eq!(c.hw.read32(reg::RDT), (RING - 1) as u32);
}

#[test]
fn sends_and_receives_frames() {
    let mut c = start(Model::new(Family::Pch), Options::DEFAULT);
    let frame: Vec<u8> = (0..100u8).collect();
    for _ in 0..(RING * 3) {
        assert!(c.send(&frame));
    }
    assert_eq!(c.hw.sent_frames.len(), RING * 3);
    assert_eq!(&c.hw.sent_frames[0][..100], &frame[..]);
    assert_eq!(c.hw.sent_frames[0].len(), 100);
    assert!(c.send(&[1, 2, 3]));
    assert_eq!(c.hw.sent_frames.last().unwrap().len(), 60, "cadrele scurte sunt completate la 60");

    let mut out = [0u8; BUF];
    assert_eq!(c.receive_into(&mut out), None);
    for k in 0..(RING * 2) {
        let f = [k as u8; 64];
        assert!(c.hw.inject_rx(&f));
        assert_eq!(c.receive_into(&mut out), Some(64));
        assert_eq!(out[0], k as u8);
    }
    assert_eq!(c.received, (RING * 2) as u64);
    assert_eq!(c.hw.dma_syncs, (RING * 2) as u32);
    assert_eq!(c.link(), (true, 1000, true));
}

#[test]
fn registers_snapshot_has_the_key_ones() {
    let mut c = start(Model::new(Family::Pch), Options::DEFAULT);
    let r = c.registers();
    assert_eq!(r[0].0, "CTRL");
    assert!(r[0].1 & CTRL_SLU != 0);
    assert!(r.iter().any(|(n, _)| *n == "EXTCNF_CTRL"));
}
