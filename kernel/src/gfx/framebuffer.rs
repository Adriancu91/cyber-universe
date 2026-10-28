//! Linear framebuffer provided by Limine (32 bits per pixel, 0x00RRGGBB).

use core::sync::atomic::{AtomicU64, Ordering};

/// A drawable view of the boot framebuffer.
pub struct Framebuffer {
    addr: *mut u8,
    pub width: usize,
    pub height: usize,
    pitch: usize,
    /// A copy of the screen in RAM (P6-29), once the heap exists: every write goes to both, reads and scrolling use
    /// the copy. Reading video memory is very slow on a real graphics card (the workstation's screen is in the RTX's
    /// BAR1, over PCIe, uncached): scrolling by copying the screen onto itself took about a second per line there.
    shadow: Option<alloc::vec::Vec<u8>>,
}

// SAFETY: the framebuffer is only accessed through the console lock (single owner).
unsafe impl Send for Framebuffer {}

/// Where Limine's first framebuffer is in physical memory, saved at boot by `from_limine` (Limine's answer lives in
/// memory the kernel reclaims later): address, width, height, pitch, bpp. Address 0 = not known.
static INFO: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

/// (physical address, size in bytes, width, height, pitch, bpp) of the boot framebuffer, as saved at boot.
/// Read only (C-26: the GPU driver must not hand out the video memory that holds the screen).
#[cfg_attr(all(feature = "qemu-test", not(feature = "test-shell")), allow(dead_code))] // `hw`
pub fn boot_info() -> Option<(u64, u64, u64, u64, u64, u16)> {
    let [phys, w, h, pitch, bpp] = [0, 1, 2, 3, 4].map(|i| INFO[i].load(Ordering::Relaxed));
    if phys == 0 {
        return None;
    }
    Some((phys, pitch * h, w, h, pitch, bpp as u16))
}

impl Framebuffer {
    /// Takes the first framebuffer from Limine. Returns `None` if there is none or it is not 32 bpp.
    pub fn from_limine() -> Option<Self> {
        let response = crate::boot::FRAMEBUFFER.get_response()?;
        let fb = response.framebuffers().next()?;
        // Limine hands the framebuffer as a higher-half direct-map address: physical = virtual - HHDM offset.
        if let Some(hhdm) = crate::boot::HHDM.get_response() {
            if let Some(phys) = (fb.addr() as u64).checked_sub(hhdm.offset()) {
                for (i, v) in [phys, fb.width(), fb.height(), fb.pitch(), fb.bpp() as u64].into_iter().enumerate() {
                    INFO[i].store(v, Ordering::Relaxed);
                }
            }
        }
        if fb.bpp() != 32 {
            crate::serial_println!("AVERTISMENT: framebuffer cu {} bpp, doar 32 bpp e suportat", fb.bpp());
            return None;
        }
        Some(Self {
            addr: fb.addr(),
            width: fb.width() as usize,
            height: fb.height() as usize,
            pitch: fb.pitch() as usize,
            shadow: None,
        })
    }

    /// Starts the RAM copy (needs the heap). Rows `0..used` are read once from the screen, the rest are `fill` (the
    /// console cleared the screen and wrote only those rows). Returns false (and keeps working without a copy) if
    /// the memory is not there.
    pub fn enable_shadow(&mut self, used: usize, fill: u32) -> bool {
        if self.shadow.is_some() {
            return true;
        }
        let len = self.pitch * self.height;
        let mut v = alloc::vec::Vec::new();
        if v.try_reserve_exact(len).is_err() {
            return false;
        }
        v.resize(len, 0);
        let used = used.min(self.height);
        // SAFETY: rows 0..used lie inside the framebuffer, `v` has room for them.
        unsafe { core::ptr::copy_nonoverlapping(self.addr, v.as_mut_ptr(), used * self.pitch) };
        for y in used..self.height {
            for x in 0..self.width {
                v[y * self.pitch + x * 4..y * self.pitch + x * 4 + 4].copy_from_slice(&fill.to_le_bytes());
            }
        }
        self.shadow = Some(v);
        true
    }

    /// Whether the RAM copy is on.
    pub fn has_shadow(&self) -> bool {
        self.shadow.is_some()
    }

    /// Sets one pixel. Coordinates outside the screen are ignored.
    #[inline]
    pub fn put_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let offset = y * self.pitch + x * 4;
        if let Some(sh) = &mut self.shadow {
            sh[offset..offset + 4].copy_from_slice(&color.to_le_bytes());
        }
        // SAFETY: (x, y) is inside the framebuffer, which Limine maps for the kernel; offset is 4-byte aligned.
        unsafe { (self.addr.add(offset) as *mut u32).write_volatile(color) }
    }

    /// Fills a rectangle, clipped to the screen.
    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        let x_end = (x + w).min(self.width);
        let y_end = (y + h).min(self.height);
        for py in y.min(self.height)..y_end {
            for px in x.min(self.width)..x_end {
                self.put_pixel(px, py, color);
            }
        }
    }

    /// Copies a row of pixels to (x, y), clipped to the screen (one memory copy: fast).
    #[allow(dead_code)] // used by `Canvas::present` (self-tests now, the window server in P4-04)
    pub fn write_row(&mut self, x: usize, y: usize, pixels: &[u32]) {
        if y >= self.height || x >= self.width {
            return;
        }
        let n = pixels.len().min(self.width - x);
        let at = y * self.pitch + x * 4;
        if let Some(sh) = &mut self.shadow {
            // SAFETY: [at, at + 4n) is inside the copy (same layout as the framebuffer); pixels are 4 bytes.
            unsafe { core::ptr::copy_nonoverlapping(pixels.as_ptr() as *const u8, sh.as_mut_ptr().add(at), n * 4) }
        }
        // SAFETY: the row segment [x, x + n) of line y is inside the framebuffer; pixels are 4 bytes.
        unsafe { core::ptr::copy_nonoverlapping(pixels.as_ptr(), self.addr.add(at) as *mut u32, n) }
    }

    /// Reads a pixel (`None` outside the screen).
    #[allow(dead_code)] // used by the self-tests
    pub fn pixel(&self, x: usize, y: usize) -> Option<u32> {
        // SAFETY: (x, y) is inside the framebuffer when the check passes; offset is 4-byte aligned.
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = y * self.pitch + x * 4;
        if let Some(sh) = &self.shadow {
            return Some(u32::from_le_bytes(sh[at..at + 4].try_into().unwrap()));
        }
        Some(unsafe { (self.addr.add(at) as *const u32).read_volatile() })
    }

    /// Fills the whole screen.
    pub fn clear(&mut self, color: u32) {
        self.fill_rect(0, 0, self.width, self.height, color);
    }

    /// Moves every pixel row up by `rows` and clears the freed area (used for scrolling).
    pub fn scroll_up(&mut self, rows: usize, color: u32) {
        if rows >= self.height {
            self.clear(color);
            return;
        }
        let bytes = (self.height - rows) * self.pitch;
        if let Some(sh) = &mut self.shadow {
            // In RAM, then written to the screen (only writes to video memory, never reads).
            sh.copy_within(rows * self.pitch..rows * self.pitch + bytes, 0);
            for y in 0..self.height - rows {
                // SAFETY: line y (width * 4 bytes) is inside both the copy and the framebuffer.
                unsafe {
                    core::ptr::copy_nonoverlapping(sh.as_ptr().add(y * self.pitch), self.addr.add(y * self.pitch), self.width * 4)
                }
            }
        } else {
            // SAFETY: both ranges lie inside the framebuffer; `copy` handles the overlap.
            unsafe { core::ptr::copy(self.addr.add(rows * self.pitch), self.addr, bytes) }
        }
        self.fill_rect(0, self.height - rows, self.width, rows, color);
    }
}
