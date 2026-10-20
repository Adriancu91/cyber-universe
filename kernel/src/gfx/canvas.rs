//! Drawing in memory ("back buffer"), then copying only what changed to the screen: no flicker, and
//! little work per frame on slow hardware. Colors are 0x00RRGGBB, like the framebuffer.

use super::font::{GLYPH_W, glyph};
use super::framebuffer::Framebuffer;
use alloc::{vec, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    pub fn new(x: usize, y: usize, w: usize, h: usize) -> Rect {
        Rect { x, y, w, h }
    }

    /// The smallest rectangle that holds both.
    pub fn union(self, o: Rect) -> Rect {
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        let (r, b) = ((self.x + self.w).max(o.x + o.w), (self.y + self.h).max(o.y + o.h));
        Rect::new(x, y, r - x, b - y)
    }

    /// This rectangle cut to a `width` x `height` area (`None` if nothing is left).
    fn clip(self, width: usize, height: usize) -> Option<Rect> {
        self.intersect(Rect::new(0, 0, width, height))
    }

    /// The common part of two rectangles (`None` if they do not overlap).
    pub fn intersect(self, o: Rect) -> Option<Rect> {
        let (x, y) = (self.x.max(o.x), self.y.max(o.y));
        let (r, b) = ((self.x + self.w).min(o.x + o.w), (self.y + self.h).min(o.y + o.h));
        (x < r && y < b).then(|| Rect::new(x, y, r - x, b - y))
    }

    pub fn contains(self, x: usize, y: usize) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pixels: Vec<u32>,
    /// Area changed since the last `present` (one rectangle around every change).
    dirty: Option<Rect>,
    /// Drawing outside this area is ignored (the window server redraws one damaged area at a time).
    clip: Option<Rect>,
}

#[allow(dead_code)] // used by the self-tests now, by the window server and the desktop (P4-04 and later)
impl Canvas {
    pub fn new(width: usize, height: usize, color: u32) -> Canvas {
        Canvas { width, height, pixels: vec![color; width * height], dirty: Some(Rect::new(0, 0, width, height)), clip: None }
    }

    /// Limits all drawing to `r` (`None` = the whole canvas).
    pub fn set_clip(&mut self, r: Option<Rect>) {
        self.clip = r;
    }

    /// `r` cut to the canvas and to the clip area.
    fn visible(&self, r: Rect) -> Option<Rect> {
        let r = r.clip(self.width, self.height)?;
        match self.clip {
            Some(c) => r.intersect(c),
            None => Some(r),
        }
    }

    fn touch(&mut self, r: Rect) {
        if let Some(r) = r.clip(self.width, self.height) {
            self.dirty = Some(self.dirty.map_or(r, |d| d.union(r)));
        }
    }

    pub fn pixel(&self, x: usize, y: usize) -> Option<u32> {
        (x < self.width && y < self.height).then(|| self.pixels[y * self.width + x])
    }

    pub fn put(&mut self, x: usize, y: usize, color: u32) {
        if self.visible(Rect::new(x, y, 1, 1)).is_some() {
            self.pixels[y * self.width + x] = color;
            self.touch(Rect::new(x, y, 1, 1));
        }
    }

    pub fn fill(&mut self, r: Rect, color: u32) {
        let Some(r) = self.visible(r) else { return };
        for y in r.y..r.y + r.h {
            self.pixels[y * self.width + r.x..y * self.width + r.x + r.w].fill(color);
        }
        self.touch(r);
    }

    /// A rectangle's border, `t` pixels thick.
    pub fn outline(&mut self, r: Rect, t: usize, color: u32) {
        self.fill(Rect::new(r.x, r.y, r.w, t), color);
        self.fill(Rect::new(r.x, (r.y + r.h).saturating_sub(t), r.w, t), color);
        self.fill(Rect::new(r.x, r.y, t, r.h), color);
        self.fill(Rect::new((r.x + r.w).saturating_sub(t), r.y, t, r.h), color);
    }

    /// A straight line between two points (Bresenham), clipped to the canvas.
    pub fn line(&mut self, x0: isize, y0: isize, x1: isize, y1: isize, color: u32) {
        let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
        let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
        let (mut x, mut y, mut err) = (x0, y0, dx + dy);
        loop {
            if x >= 0 && y >= 0 {
                self.put(x as usize, y as usize, color);
            }
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Copies a `w` x `h` image to (x, y); pixels equal to `transparent` (if given) are skipped.
    pub fn blit(&mut self, x: usize, y: usize, w: usize, h: usize, image: &[u32], transparent: Option<u32>) {
        let Some(v) = self.visible(Rect::new(x, y, w, h)) else { return };
        for row in v.y - y..v.y - y + v.h {
            for col in v.x - x..v.x - x + v.w {
                let c = image[row * w + col];
                if Some(c) != transparent {
                    self.pixels[(y + row) * self.width + x + col] = c;
                }
            }
        }
        self.touch(v);
    }

    /// Copies a `w` x `h` opaque image to (x, y) row by row (fast path for window contents).
    pub fn copy(&mut self, x: usize, y: usize, w: usize, h: usize, image: &[u32]) {
        let Some(v) = self.visible(Rect::new(x, y, w, h)) else { return };
        let (cx, cy) = (v.x - x, v.y - y);
        for row in 0..v.h {
            let src = (cy + row) * w + cx;
            let dst = (v.y + row) * self.width + v.x;
            self.pixels[dst..dst + v.w].copy_from_slice(&image[src..src + v.w]);
        }
        self.touch(v);
    }

    /// Text with the built-in font, each font pixel drawn as `scale` x `scale` pixels. Returns the width drawn.
    pub fn text(&mut self, x: usize, y: usize, s: &str, color: u32, scale: usize) -> usize {
        let advance = (GLYPH_W + 1) * scale;
        for (i, c) in s.chars().enumerate() {
            let x0 = x + i * advance;
            for (gy, bits) in glyph(c).iter().enumerate() {
                for gx in 0..GLYPH_W {
                    if bits & (1 << (GLYPH_W - 1 - gx)) != 0 {
                        self.fill(Rect::new(x0 + gx * scale, y + gy * scale, scale, scale), color);
                    }
                }
            }
        }
        s.chars().count() * advance
    }

    /// Forgets the changed area and returns it (the window server turns it into screen damage).
    pub fn take_dirty(&mut self) -> Option<Rect> {
        self.dirty.take()
    }

    /// All pixels, row by row.
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// Copies the changed area to the screen at (`at_x`, `at_y`) and forgets it. Returns what was copied.
    pub fn present(&mut self, fb: &mut Framebuffer, at_x: usize, at_y: usize) -> Option<Rect> {
        let d = self.dirty.take()?;
        for y in d.y..d.y + d.h {
            fb.write_row(at_x + d.x, at_y + y, &self.pixels[y * self.width + d.x..y * self.width + d.x + d.w]);
        }
        Some(d)
    }
}
