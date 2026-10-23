//! A software model of an Intel gigabit card, enough to test the driver logic on the host: registers, the MDIC PHY
//! interface, the PCH semaphore and LANPHYPC pin, reset, and descriptor DMA. Also records rule violations
//! (PHY access without the semaphore on PCH, reset with the rings running).

use crate::{reg, Family, Hw};
use std::collections::HashMap;
use std::vec::Vec;

pub struct Model {
    pub family: Family,
    pub regs: HashMap<u32, u32>,
    /// MAC loaded from the NVM at reset.
    pub nvm_mac: Option<[u8; 6]>,
    /// The PHY is in its low-power mode and does not answer until a LANPHYPC power cycle.
    pub phy_asleep: bool,
    /// The firmware holds the semaphore forever.
    pub fw_holds_semaphore: bool,
    /// The reset bit never clears.
    pub reset_stuck: bool,
    /// LAN_INIT_DONE never comes after a reset.
    pub no_lan_init_done: bool,
    /// Every register reads as all ones.
    pub all_ones: bool,
    /// PHY id (ID1, ID2).
    pub phy_id: (u16, u16),
    /// The firmware left the rings running.
    pub rings_running: bool,
    pub violations: Vec<&'static str>,
    pub resets: u32,
    pub lanphypc_cycles: u32,
    pub dma_syncs: u32,
    pub sent_frames: Vec<Vec<u8>>,
    pub delay_total_us: u64,
    block: Option<(u64, *mut u8)>,
    override_low: bool,
    rx_fill: usize,
}

impl Model {
    pub fn new(family: Family) -> Model {
        let mut m = Model {
            family,
            regs: HashMap::new(),
            nvm_mac: Some([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]),
            phy_asleep: false,
            fw_holds_semaphore: false,
            reset_stuck: false,
            no_lan_init_done: false,
            all_ones: false,
            phy_id: if family == Family::Pch { (0x0154, 0x00A0) } else { (0x0141, 0x0CB0) },
            rings_running: false,
            violations: Vec::new(),
            resets: 0,
            lanphypc_cycles: 0,
            dma_syncs: 0,
            sent_frames: Vec::new(),
            delay_total_us: 0,
            block: None,
            override_low: false,
            rx_fill: 0,
        };
        m.load_nvm();
        m
    }

    fn load_nvm(&mut self) {
        if let Some(mac) = self.nvm_mac {
            let ral = u32::from_le_bytes([mac[0], mac[1], mac[2], mac[3]]);
            let rah = u16::from_le_bytes([mac[4], mac[5]]) as u32 | crate::RAH_AV;
            self.regs.insert(reg::RAL0, ral);
            self.regs.insert(reg::RAH0, rah);
        }
    }

    fn get(&self, r: u32) -> u32 {
        *self.regs.get(&r).unwrap_or(&0)
    }

    fn semaphore_held(&self) -> bool {
        self.get(reg::EXTCNF_CTRL) & crate::EXTCNF_SWFLAG != 0
    }

    fn reset(&mut self) {
        self.resets += 1;
        if self.get(reg::RCTL) & crate::RCTL_EN != 0 || self.get(reg::TCTL) & crate::TCTL_EN != 0 {
            self.violations.push("reset cu RX/TX pornite");
        }
        self.regs.clear();
        self.load_nvm();
        if self.reset_stuck {
            self.regs.insert(reg::CTRL, crate::CTRL_RST);
        }
        if self.family == Family::Pch && !self.no_lan_init_done {
            self.regs.insert(reg::STATUS, crate::STATUS_LAN_INIT_DONE);
        }
    }

    /// The card receives a frame: written into the next RX buffer, descriptor marked done.
    pub fn inject_rx(&mut self, frame: &[u8]) -> bool {
        let Some((phys, ptr)) = self.block else { return false };
        let base = (self.get(reg::RDBAL) as u64) | (self.get(reg::RDBAH) as u64) << 32;
        if base != phys || self.get(reg::RCTL) & crate::RCTL_EN == 0 {
            return false;
        }
        let i = self.rx_fill;
        // SAFETY: the model owns the block; descriptor i and its buffer lie inside it.
        unsafe {
            let d = (ptr as *mut u64).add(i * 2);
            let buf = d.read();
            let b = ptr.add((buf - phys) as usize);
            std::ptr::copy_nonoverlapping(frame.as_ptr(), b, frame.len());
            d.add(1).write(frame.len() as u64 | 1 << 32);
        }
        self.rx_fill = (i + 1) % crate::RING;
        true
    }

    fn transmit(&mut self, tdt: u32) {
        let Some((phys, ptr)) = self.block else { return };
        let base = (self.get(reg::TDBAL) as u64) | (self.get(reg::TDBAH) as u64) << 32;
        let mut h = self.get(reg::TDH) as usize;
        while h != tdt as usize {
            // SAFETY: descriptor h of the TX ring inside the model's block.
            unsafe {
                let d = ptr.add((base - phys) as usize) as *mut u64;
                let d = d.add(h * 2);
                let buf = d.read();
                let w = d.add(1).read();
                let len = (w & 0xFFFF) as usize;
                let b = ptr.add((buf - phys) as usize);
                self.sent_frames.push(std::slice::from_raw_parts(b, len).to_vec());
                d.add(1).write(w | 1 << 32);
            }
            h = (h + 1) % crate::RING;
        }
        self.regs.insert(reg::TDH, h as u32);
    }
}

impl Hw for Model {
    fn read32(&mut self, r: u32) -> u32 {
        if self.all_ones {
            return u32::MAX;
        }
        let v = self.get(r);
        match r {
            reg::STATUS => {
                let lu = if self.get(reg::CTRL) & crate::CTRL_SLU != 0 && !self.phy_asleep { crate::STATUS_LU | crate::STATUS_FD | 2 << 6 } else { 0 };
                v | lu
            }
            reg::CTRL if !self.reset_stuck => v & !crate::CTRL_RST,
            reg::EXTCNF_CTRL if self.fw_holds_semaphore => v & !crate::EXTCNF_SWFLAG,
            reg::RCTL if self.rings_running => v | crate::RCTL_EN,
            _ => v,
        }
    }

    fn write32(&mut self, r: u32, v: u32) {
        match r {
            reg::CTRL => {
                if v & crate::CTRL_RST != 0 {
                    self.reset();
                    return;
                }
                let low = v & crate::CTRL_LANPHYPC_OVERRIDE != 0 && v & crate::CTRL_LANPHYPC_VALUE == 0;
                if self.override_low && v & crate::CTRL_LANPHYPC_OVERRIDE == 0 {
                    self.lanphypc_cycles += 1;
                    self.phy_asleep = false;
                    let e = self.get(reg::CTRL_EXT);
                    self.regs.insert(reg::CTRL_EXT, e | crate::CTRL_EXT_LPCD);
                }
                self.override_low = low;
                self.regs.insert(r, v);
            }
            reg::MDIC => {
                if self.family == Family::Pch && !self.semaphore_held() {
                    self.violations.push("MDIC fara semafor");
                }
                let phy = ((v >> 21) & 0x1F) as u8;
                let regn = ((v >> 16) & 0x1F) as u8;
                let data = if phy != self.family.phy_address() || self.phy_asleep {
                    0xFFFF
                } else {
                    match regn {
                        crate::PHY_ID1 => self.phy_id.0,
                        crate::PHY_ID2 => self.phy_id.1,
                        crate::PHY_BMSR => 0x796D,
                        _ => 0,
                    }
                };
                self.regs.insert(r, crate::MDIC_READY | (v & 0x0FFF_0000) | data as u32);
            }
            reg::STATUS => {
                // only LAN_INIT_DONE is writable (write 0 to acknowledge)
                let s = self.get(r);
                self.regs.insert(r, (s & !crate::STATUS_LAN_INIT_DONE) | (v & s & crate::STATUS_LAN_INIT_DONE));
            }
            reg::RCTL => {
                self.rings_running = false;
                self.regs.insert(r, v);
            }
            reg::TDT => {
                self.regs.insert(r, v);
                if self.get(reg::TCTL) & crate::TCTL_EN != 0 {
                    self.transmit(v);
                }
            }
            _ => {
                self.regs.insert(r, v);
            }
        }
    }

    fn alloc_block(&mut self, pages: usize) -> Option<(u64, *mut u8)> {
        let layout = std::alloc::Layout::from_size_align(pages * 4096, 4096).ok()?;
        // SAFETY: non-zero size; the block lives for the whole test (leaked on purpose).
        let p = unsafe { std::alloc::alloc_zeroed(layout) };
        if p.is_null() {
            return None;
        }
        let phys = 0x0200_0000; // 32 MiB: between 16 MiB and 4 GiB, as the kernel gives
        self.block = Some((phys, p));
        Some((phys, p))
    }

    fn delay_us(&mut self, us: u32) {
        self.delay_total_us += us as u64;
    }

    fn dma_sync(&mut self, _ptr: *mut u8, _len: usize) {
        self.dma_syncs += 1;
    }
}
