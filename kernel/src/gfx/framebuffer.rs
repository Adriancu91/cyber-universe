//! Linear framebuffer provided by Limine (32 bits per pixel, 0x00RRGGBB).

use core::sync::atomic::{AtomicU64, Ordering};

/// A drawable view of the boot framebuffer.
pub struct Framebuffer {
    addr: *mut u8,
    pub width: usize,
    pub height: usize,
    pitch: usize,
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
        })
    }

    /// Sets one pixel. Coordinates outside the screen are ignored.
    #[inline]
    pub fn put_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let offset = y * self.pitch + x * 4;
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
        // SAFETY: the row segment [x, x + n) of line y is inside the framebuffer; pixels are 4 bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(pixels.as_ptr(), self.addr.add(y * self.pitch + x * 4) as *mut u32, n)
        }
    }

    /// Reads a pixel (`None` outside the screen).
    #[allow(dead_code)] // used by the self-tests
    pub fn pixel(&self, x: usize, y: usize) -> Option<u32> {
        // SAFETY: (x, y) is inside the framebuffer when the check passes; offset is 4-byte aligned.
        (x < self.width && y < self.height)
            .then(|| unsafe { (self.addr.add(y * self.pitch + x * 4) as *const u32).read_volatile() })
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
        // SAFETY: both ranges lie inside the framebuffer; `copy` handles the overlap.
        unsafe { core::ptr::copy(self.addr.add(rows * self.pitch), self.addr, bytes) }
        self.fill_rect(0, self.height - rows, self.width, rows, color);
    }
}
