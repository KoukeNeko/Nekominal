//! Procedural glyphs drawn at the exact integer cell size.
//!
//! Box drawing, block elements, braille, Powerline separators and the curly /
//! dotted / dashed underlines are rasterized on the CPU into R8 coverage
//! bitmaps that cover the whole cell, so neighbouring cells join without gaps
//! regardless of the font. The straight-line box drawing logic is a port of
//! Ghostty's `linesChar` (MIT); everything else is original.

use crate::cell::Underline;
use crate::fonts::CellMetrics;

pub struct SpriteBitmap {
    pub w: u32,
    pub h: u32,
    /// R8 coverage, `w * h`, placed at the cell origin.
    pub data: Vec<u8>,
}

struct Canvas {
    w: u32,
    h: u32,
    data: Vec<u8>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        Canvas { w, h, data: vec![0; (w * h) as usize] }
    }

    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let (x0, x1) = (x0.clamp(0, self.w as i32), x1.clamp(0, self.w as i32));
        let (y0, y1) = (y0.clamp(0, self.h as i32), y1.clamp(0, self.h as i32));
        for y in y0..y1 {
            for x in x0..x1 {
                self.data[(y as u32 * self.w + x as u32) as usize] = 255;
            }
        }
    }

    fn set_max(&mut self, x: u32, y: u32, v: u8) {
        let p = &mut self.data[(y * self.w + x) as usize];
        *p = (*p).max(v);
    }

    fn fill_all(&mut self, v: u8) {
        self.data.fill(v);
    }

    /// Anti-aliased fill of a convex or concave polygon by 8x8 supersampling.
    fn polygon(&mut self, pts: &[(f32, f32)]) {
        const SS: u32 = 8;
        let (mut miny, mut maxy, mut minx, mut maxx) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for &(x, y) in pts {
            miny = miny.min(y);
            maxy = maxy.max(y);
            minx = minx.min(x);
            maxx = maxx.max(x);
        }
        let (x0, x1) = (minx.floor().max(0.0) as u32, (maxx.ceil() as u32).min(self.w));
        let (y0, y1) = (miny.floor().max(0.0) as u32, (maxy.ceil() as u32).min(self.h));
        for py in y0..y1 {
            for px in x0..x1 {
                let mut hit = 0u32;
                for sy in 0..SS {
                    for sx in 0..SS {
                        let x = px as f32 + (sx as f32 + 0.5) / SS as f32;
                        let y = py as f32 + (sy as f32 + 0.5) / SS as f32;
                        if point_in_poly(x, y, pts) {
                            hit += 1;
                        }
                    }
                }
                if hit > 0 {
                    self.set_max(px, py, (hit * 255 / (SS * SS)) as u8);
                }
            }
        }
    }

    /// Stroke of constant thickness along a polyline using a distance field
    /// (butt ends, round joins are not needed for our shapes).
    fn stroke(&mut self, pts: &[(f32, f32)], thickness: f32) {
        let half = thickness * 0.5;
        for py in 0..self.h {
            for px in 0..self.w {
                let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
                let mut d = f32::MAX;
                for seg in pts.windows(2) {
                    d = d.min(dist_to_segment(x, y, seg[0], seg[1]));
                }
                let cov = (half + 0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.set_max(px, py, (cov * 255.0) as u8);
                }
            }
        }
    }

    fn disc(&mut self, cx: f32, cy: f32, r: f32) {
        let (x0, x1) = ((cx - r - 1.0).floor().max(0.0) as u32, ((cx + r + 1.0).ceil() as u32).min(self.w));
        let (y0, y1) = ((cy - r - 1.0).floor().max(0.0) as u32, ((cy + r + 1.0).ceil() as u32).min(self.h));
        for py in y0..y1 {
            for px in x0..x1 {
                let d = ((px as f32 + 0.5 - cx).powi(2) + (py as f32 + 0.5 - cy).powi(2)).sqrt();
                let cov = (r + 0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.set_max(px, py, (cov * 255.0) as u8);
                }
            }
        }
    }

    fn finish(self) -> SpriteBitmap {
        SpriteBitmap { w: self.w, h: self.h, data: self.data }
    }
}

fn point_in_poly(x: f32, y: f32, pts: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn dist_to_segment(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0) };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub fn is_sprite_char(ch: char) -> bool {
    matches!(ch as u32, 0x2500..=0x259F | 0x2800..=0x28FF | 0xE0B0..=0xE0B8 | 0xE0B9..=0xE0BF)
}

pub fn draw_char(ch: char, m: &CellMetrics) -> Option<SpriteBitmap> {
    let mut c = Canvas::new(m.width, m.height);
    match ch as u32 {
        0x2500..=0x257F => box_drawing(&mut c, ch as u32, m)?,
        0x2580..=0x259F => block_element(&mut c, ch as u32),
        0x2800..=0x28FF => braille(&mut c, ch as u32),
        0xE0B0..=0xE0BF => powerline(&mut c, ch as u32, m)?,
        _ => return None,
    }
    Some(c.finish())
}

// ---------------------------------------------------------------------------
// Box drawing 2500..=257F
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum W {
    None,
    Light,
    Heavy,
    Double,
}

/// Arms in (up, right, down, left) order: 0 none, 1 light, 2 heavy, 3 double.
/// Dashed (0x04-0x0B, 0x4C-0x4F), arcs (0x6D-0x70) and diagonals (0x71-0x73)
/// are handled separately and have a zero row here.
#[rustfmt::skip]
const ARMS: [[u8; 4]; 128] = [
    [0,1,0,1],[0,2,0,2],[1,0,1,0],[2,0,2,0],           // 2500-2503
    [0;4],[0;4],[0;4],[0;4],[0;4],[0;4],[0;4],[0;4],   // 2504-250B dashes
    [0,1,1,0],[0,2,1,0],[0,1,2,0],[0,2,2,0],           // 250C-250F
    [0,0,1,1],[0,0,1,2],[0,0,2,1],[0,0,2,2],           // 2510-2513
    [1,1,0,0],[1,2,0,0],[2,1,0,0],[2,2,0,0],           // 2514-2517
    [1,0,0,1],[1,0,0,2],[2,0,0,1],[2,0,0,2],           // 2518-251B
    [1,1,1,0],[1,2,1,0],[2,1,1,0],[1,1,2,0],           // 251C-251F
    [2,1,2,0],[2,2,1,0],[1,2,2,0],[2,2,2,0],           // 2520-2523
    [1,0,1,1],[1,0,1,2],[2,0,1,1],[1,0,2,1],           // 2524-2527
    [2,0,2,1],[2,0,1,2],[1,0,2,2],[2,0,2,2],           // 2528-252B
    [0,1,1,1],[0,1,1,2],[0,2,1,1],[0,2,1,2],           // 252C-252F
    [0,1,2,1],[0,1,2,2],[0,2,2,1],[0,2,2,2],           // 2530-2533
    [1,1,0,1],[1,1,0,2],[1,2,0,1],[1,2,0,2],           // 2534-2537
    [2,1,0,1],[2,1,0,2],[2,2,0,1],[2,2,0,2],           // 2538-253B
    [1,1,1,1],[1,1,1,2],[1,2,1,1],[1,2,1,2],           // 253C-253F
    [2,1,1,1],[1,1,2,1],[2,1,2,1],[2,1,1,2],           // 2540-2543
    [2,2,1,1],[1,1,2,2],[1,2,2,1],[2,2,1,2],           // 2544-2547
    [1,2,2,2],[2,1,2,2],[2,2,2,1],[2,2,2,2],           // 2548-254B
    [0;4],[0;4],[0;4],[0;4],                           // 254C-254F dashes
    [0,3,0,3],[3,0,3,0],[0,3,1,0],[0,1,3,0],           // 2550-2553
    [0,3,3,0],[0,0,1,3],[0,0,3,1],[0,0,3,3],           // 2554-2557
    [1,3,0,0],[3,1,0,0],[3,3,0,0],[1,0,0,3],           // 2558-255B
    [3,0,0,1],[3,0,0,3],[1,3,1,0],[3,1,3,0],           // 255C-255F
    [3,3,3,0],[1,0,1,3],[3,0,3,1],[3,0,3,3],           // 2560-2563
    [0,3,1,3],[0,1,3,1],[0,3,3,3],[1,3,0,3],           // 2564-2567
    [3,1,0,1],[3,3,0,3],[1,3,1,3],[3,1,3,1],           // 2568-256B
    [3,3,3,3],[0;4],[0;4],[0;4],                       // 256C, 256D-256F arcs
    [0;4],[0;4],[0;4],[0;4],                           // 2570 arc, 2571-2573 diagonals
    [0,0,0,1],[1,0,0,0],[0,1,0,0],[0,0,1,0],           // 2574-2577
    [0,0,0,2],[2,0,0,0],[0,2,0,0],[0,0,2,0],           // 2578-257B
    [0,2,0,1],[1,0,2,0],[0,1,0,2],[2,0,1,0],           // 257C-257F
];

fn weight(v: u8) -> W {
    match v {
        1 => W::Light,
        2 => W::Heavy,
        3 => W::Double,
        _ => W::None,
    }
}

fn box_drawing(c: &mut Canvas, cp: u32, m: &CellMetrics) -> Option<()> {
    let light = m.underline_thickness.max(1) as i32;
    let heavy = light * 2;
    match cp {
        0x2504..=0x250B | 0x254C..=0x254F => {
            let (n, vertical, thick) = match cp {
                0x2504 => (3, false, light),
                0x2505 => (3, false, heavy),
                0x2506 => (3, true, light),
                0x2507 => (3, true, heavy),
                0x2508 => (4, false, light),
                0x2509 => (4, false, heavy),
                0x250A => (4, true, light),
                0x250B => (4, true, heavy),
                0x254C => (2, false, light),
                0x254D => (2, false, heavy),
                0x254E => (2, true, light),
                _ => (2, true, heavy),
            };
            dashes(c, n, vertical, thick);
        }
        0x256D..=0x2570 => arc(c, cp, light as f32),
        0x2571..=0x2573 => {
            let (w, h) = (c.w as f32, c.h as f32);
            let sx = (w / h).min(1.0);
            let sy = (h / w).min(1.0);
            if cp != 0x2572 {
                c.stroke(&[(w + 0.5 * sx, -0.5 * sy), (-0.5 * sx, h + 0.5 * sy)], light as f32);
            }
            if cp != 0x2571 {
                c.stroke(&[(-0.5 * sx, -0.5 * sy), (w + 0.5 * sx, h + 0.5 * sy)], light as f32);
            }
        }
        _ => {
            let a = ARMS[(cp - 0x2500) as usize];
            lines(c, [weight(a[0]), weight(a[1]), weight(a[2]), weight(a[3])], light, heavy);
        }
    }
    Some(())
}

fn dashes(c: &mut Canvas, n: i32, vertical: bool, thick: i32) {
    let len = if vertical { c.h as i32 } else { c.w as i32 };
    let gap = (len / (n * 2)).max(1);
    let center = |total: i32| (total - thick) / 2;
    for i in 0..n {
        let a = i * len / n + gap / 2;
        let b = ((i + 1) * len / n - (gap - gap / 2)).max(a + 1);
        if vertical {
            let x0 = center(c.w as i32);
            c.rect(x0, a, x0 + thick, b);
        } else {
            let y0 = center(c.h as i32);
            c.rect(a, y0, b, y0 + thick);
        }
    }
}

/// Port of Ghostty's `linesChar`: rect-based, pixel exact joins for light,
/// heavy and double strokes (including mixed corners and tees).
fn lines(c: &mut Canvas, arms: [W; 4], light: i32, heavy: i32) {
    let [up, right, down, left] = arms;
    let (cw, ch) = (c.w as i32, c.h as i32);
    let h_light_top = (ch - light).max(0) / 2;
    let h_light_bottom = h_light_top + light;
    let h_heavy_top = (ch - heavy).max(0) / 2;
    let h_heavy_bottom = h_heavy_top + heavy;
    let h_double_top = h_light_top - light;
    let h_double_bottom = h_light_bottom + light;
    let v_light_left = (cw - light).max(0) / 2;
    let v_light_right = v_light_left + light;
    let v_heavy_left = (cw - heavy).max(0) / 2;
    let v_heavy_right = v_heavy_left + heavy;
    let v_double_left = v_light_left - light;
    let v_double_right = v_light_right + light;

    let up_bottom = if left == W::Heavy || right == W::Heavy {
        h_heavy_bottom
    } else if left != right || down == up {
        if left == W::Double || right == W::Double { h_double_bottom } else { h_light_bottom }
    } else if left == W::None && right == W::None {
        h_light_bottom
    } else {
        h_light_top
    };
    let down_top = if left == W::Heavy || right == W::Heavy {
        h_heavy_top
    } else if left != right || up == down {
        if left == W::Double || right == W::Double { h_double_top } else { h_light_top }
    } else if left == W::None && right == W::None {
        h_light_top
    } else {
        h_light_bottom
    };
    let left_right = if up == W::Heavy || down == W::Heavy {
        v_heavy_right
    } else if up != down || left == right {
        if up == W::Double || down == W::Double { v_double_right } else { v_light_right }
    } else if up == W::None && down == W::None {
        v_light_right
    } else {
        v_light_left
    };
    let right_left = if up == W::Heavy || down == W::Heavy {
        v_heavy_left
    } else if up != down || right == left {
        if up == W::Double || down == W::Double { v_double_left } else { v_light_left }
    } else if up == W::None && down == W::None {
        v_light_left
    } else {
        v_light_right
    };

    match up {
        W::None => {}
        W::Light => c.rect(v_light_left, 0, v_light_right, up_bottom),
        W::Heavy => c.rect(v_heavy_left, 0, v_heavy_right, up_bottom),
        W::Double => {
            let lb = if left == W::Double { h_light_top } else { up_bottom };
            let rb = if right == W::Double { h_light_top } else { up_bottom };
            c.rect(v_double_left, 0, v_light_left, lb);
            c.rect(v_light_right, 0, v_double_right, rb);
        }
    }
    match right {
        W::None => {}
        W::Light => c.rect(right_left, h_light_top, cw, h_light_bottom),
        W::Heavy => c.rect(right_left, h_heavy_top, cw, h_heavy_bottom),
        W::Double => {
            let tl = if up == W::Double { v_light_right } else { right_left };
            let bl = if down == W::Double { v_light_right } else { right_left };
            c.rect(tl, h_double_top, cw, h_light_top);
            c.rect(bl, h_light_bottom, cw, h_double_bottom);
        }
    }
    match down {
        W::None => {}
        W::Light => c.rect(v_light_left, down_top, v_light_right, ch),
        W::Heavy => c.rect(v_heavy_left, down_top, v_heavy_right, ch),
        W::Double => {
            let lt = if left == W::Double { h_light_bottom } else { down_top };
            let rt = if right == W::Double { h_light_bottom } else { down_top };
            c.rect(v_double_left, lt, v_light_left, ch);
            c.rect(v_light_right, rt, v_double_right, ch);
        }
    }
    match left {
        W::None => {}
        W::Light => c.rect(0, h_light_top, left_right, h_light_bottom),
        W::Heavy => c.rect(0, h_heavy_top, left_right, h_heavy_bottom),
        W::Double => {
            let tr = if up == W::Double { v_light_left } else { left_right };
            let br = if down == W::Double { v_light_left } else { left_right };
            c.rect(0, h_double_top, tr, h_light_top);
            c.rect(0, h_light_bottom, br, h_double_bottom);
        }
    }
}

fn arc(c: &mut Canvas, cp: u32, thick: f32) {
    let (w, h) = (c.w as f32, c.h as f32);
    let t = thick.round().max(1.0);
    let cx = ((w - t) / 2.0).floor() + t / 2.0;
    let cy = ((h - t) / 2.0).floor() + t / 2.0;
    let r = (w.min(h) / 2.0).min(cx.min(w - cx)).min(cy.min(h - cy)).max(1.0);
    // Corner is described by the two directions the stroke leaves the center.
    // (sx, sy): horizontal/vertical sign toward the open edges.
    let (sx, sy) = match cp {
        0x256D => (1.0, 1.0),   // down + right
        0x256E => (-1.0, 1.0),  // down + left
        0x256F => (-1.0, -1.0), // up + left
        _ => (1.0, -1.0),       // up + right
    };
    let (ox, oy) = (cx + sx * r, cy + sy * r); // center of curvature
    let mut pts = vec![(cx, if sy > 0.0 { h } else { 0.0 }), (cx, cy + sy * r)];
    const STEPS: usize = 24;
    for i in 1..STEPS {
        let a = (i as f32 / STEPS as f32) * std::f32::consts::FRAC_PI_2;
        // Sweep from the vertical stem end (phi=0) to the horizontal stem start (phi=pi/2).
        pts.push((ox - sx * r * a.cos(), oy - sy * r * a.sin()));
    }
    pts.push((cx + sx * r, cy));
    pts.push((if sx > 0.0 { w } else { 0.0 }, cy));
    c.stroke(&pts, t);
}

// ---------------------------------------------------------------------------
// Block elements 2580..=259F
// ---------------------------------------------------------------------------

fn block_element(c: &mut Canvas, cp: u32) {
    let (w, h) = (c.w as i32, c.h as i32);
    let eighths = |n: i32, total: i32| (n * total + 4) / 8;
    match cp {
        0x2580 => c.rect(0, 0, w, h / 2),
        0x2581..=0x2587 => {
            let n = eighths((cp - 0x2580) as i32, h);
            c.rect(0, h - n, w, h);
        }
        0x2588 => c.rect(0, 0, w, h),
        0x2589..=0x258F => {
            let n = eighths(8 - (cp - 0x2588) as i32, w);
            c.rect(0, 0, n, h);
        }
        0x2590 => c.rect(w / 2, 0, w, h),
        0x2591 => c.fill_all(64),
        0x2592 => c.fill_all(128),
        0x2593 => c.fill_all(192),
        0x2594 => c.rect(0, 0, w, eighths(1, h).max(1)),
        0x2595 => c.rect(w - eighths(1, w).max(1), 0, w, h),
        _ => {
            // Quadrants: bit 1 UL, 2 UR, 4 LL, 8 LR.
            let q = match cp {
                0x2596 => 4,
                0x2597 => 8,
                0x2598 => 1,
                0x2599 => 1 | 4 | 8,
                0x259A => 1 | 8,
                0x259B => 1 | 2 | 4,
                0x259C => 1 | 2 | 8,
                0x259D => 2,
                0x259E => 2 | 4,
                _ => 2 | 4 | 8,
            };
            let (mx, my) = (w / 2, h / 2);
            if q & 1 != 0 {
                c.rect(0, 0, mx, my);
            }
            if q & 2 != 0 {
                c.rect(mx, 0, w, my);
            }
            if q & 4 != 0 {
                c.rect(0, my, mx, h);
            }
            if q & 8 != 0 {
                c.rect(mx, my, w, h);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Braille 2800..=28FF
// ---------------------------------------------------------------------------

fn braille(c: &mut Canvas, cp: u32) {
    let bits = cp - 0x2800;
    let (w, h) = (c.w as f32, c.h as f32);
    let r = (w * 0.17).min(h * 0.085).max(1.0);
    // dot index (bit) -> (column, row)
    const POS: [(u32, u32); 8] = [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)];
    for (bit, &(col, row)) in POS.iter().enumerate() {
        if bits & (1 << bit) != 0 {
            let x = w * (0.25 + 0.5 * col as f32);
            let y = h * (0.125 + 0.25 * row as f32);
            c.disc(x, y, r);
        }
    }
}

// ---------------------------------------------------------------------------
// Powerline E0B0..=E0BF
// ---------------------------------------------------------------------------

fn powerline(c: &mut Canvas, cp: u32, m: &CellMetrics) -> Option<()> {
    let (w, h) = (c.w as f32, c.h as f32);
    let t = m.underline_thickness.max(1) as f32;
    match cp {
        0xE0B0 => c.polygon(&[(0.0, 0.0), (w, h / 2.0), (0.0, h)]),
        0xE0B1 => c.stroke(&[(0.0, 0.0), (w, h / 2.0), (0.0, h)], t),
        0xE0B2 => c.polygon(&[(w, 0.0), (0.0, h / 2.0), (w, h)]),
        0xE0B3 => c.stroke(&[(w, 0.0), (0.0, h / 2.0), (w, h)], t),
        0xE0B4 | 0xE0B6 => {
            let mirror = cp == 0xE0B6;
            let mut pts = Vec::new();
            const N: usize = 48;
            for i in 0..=N {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / N as f32;
                let x = w * a.cos();
                let y = h / 2.0 + (h / 2.0) * a.sin();
                pts.push((if mirror { w - x } else { x }, y));
            }
            c.polygon(&pts);
        }
        0xE0B5 | 0xE0B7 => {
            let mirror = cp == 0xE0B7;
            let mut pts = Vec::new();
            const N: usize = 48;
            for i in 0..=N {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / N as f32;
                let x = (w - t * 0.5) * a.cos();
                let y = h / 2.0 + (h / 2.0 - t * 0.5) * a.sin();
                pts.push((if mirror { w - x } else { x }, y));
            }
            c.stroke(&pts, t);
        }
        0xE0B8 => c.polygon(&[(0.0, 0.0), (0.0, h), (w, h)]),
        0xE0B9 => c.stroke(&[(0.0, 0.0), (w, h)], t),
        0xE0BA => c.polygon(&[(w, 0.0), (w, h), (0.0, h)]),
        0xE0BB => c.stroke(&[(w, 0.0), (0.0, h)], t),
        0xE0BC => c.polygon(&[(0.0, 0.0), (w, 0.0), (0.0, h)]),
        0xE0BD => c.stroke(&[(w, 0.0), (0.0, h)], t),
        0xE0BE => c.polygon(&[(0.0, 0.0), (w, 0.0), (w, h)]),
        0xE0BF => c.stroke(&[(0.0, 0.0), (w, h)], t),
        _ => return None,
    }
    Some(())
}

// ---------------------------------------------------------------------------
// Underline styles (cell wide so runs of cells join seamlessly)
// ---------------------------------------------------------------------------

pub fn draw_underline(kind: Underline, m: &CellMetrics) -> Option<SpriteBitmap> {
    let mut c = Canvas::new(m.width, m.height);
    let (w, h) = (m.width as f32, m.height as f32);
    let t = m.underline_thickness as f32;
    // Keep decorations inside the cell: reserve room for the amplitude.
    let amp = (t * 1.5).max(1.5).min((h - m.underline_top as f32 - t).max(0.0) * 0.5 + 1.0);
    let y0 = (m.underline_top as f32 + t * 0.5).min(h - amp - 0.5 - t * 0.5);
    match kind {
        Underline::Curly => {
            let mut pts = Vec::new();
            const N: usize = 40;
            for i in 0..=N {
                let x = w * i as f32 / N as f32;
                // Period = one cell, phase chosen so x=0 and x=w both sit at the
                // center line and the curve is continuous across cells.
                let y = y0 + amp * (std::f32::consts::TAU * x / w).sin() * 0.5 + amp * 0.5;
                pts.push((x, y));
            }
            // Extend a hair beyond the cell so neighbours overlap instead of leaving a seam.
            let first = pts[0];
            let last = *pts.last().unwrap();
            pts.insert(0, (first.0 - 0.5, first.1));
            pts.push((last.0 + 0.5, last.1));
            c.stroke(&pts, t);
        }
        Underline::Dotted => {
            let r = (t * 0.5).max(0.5);
            let y = m.underline_top as f32 + t * 0.5;
            let n = 2.0;
            for i in 0..2 {
                let x = w * (i as f32 + 0.5) / n;
                c.disc(x, y, r.max(0.8));
            }
        }
        Underline::Dashed => {
            let y0 = m.underline_top as i32;
            let len = (w * 0.75).round() as i32;
            c.rect(0, y0, len, y0 + t as i32);
        }
        _ => return None,
    }
    Some(c.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> CellMetrics {
        CellMetrics {
            width: 9,
            height: 19,
            baseline: 15,
            underline_top: 17,
            underline_thickness: 1,
            strike_top: 10,
            strike_thickness: 1,
            px_size: 16.0,
            advance: 9.0,
            ascent: 15.0,
            descent: 4.0,
            leading: 0.0,
        }
    }

    #[test]
    fn full_block_covers_cell() {
        let s = draw_char('\u{2588}', &metrics()).unwrap();
        assert!(s.data.iter().all(|&v| v == 255));
    }

    #[test]
    fn horizontal_line_reaches_both_edges() {
        let m = metrics();
        let s = draw_char('\u{2500}', &m).unwrap();
        let row = (m.height as usize - 1) / 2;
        assert_eq!(s.data[row * m.width as usize], 255);
        assert_eq!(s.data[row * m.width as usize + m.width as usize - 1], 255);
    }

    #[test]
    fn upper_and_lower_half_tile_the_cell() {
        let m = metrics();
        let up = draw_char('\u{2580}', &m).unwrap();
        let lo = draw_char('\u{2584}', &m).unwrap();
        for i in 0..up.data.len() {
            let sum = up.data[i] as u32 + lo.data[i] as u32;
            assert!(sum >= 255, "pixel {i} uncovered");
        }
    }
}
