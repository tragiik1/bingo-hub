//! The logo, "Bingo of Grace": a 3x3 card fills in square by square, each one stamping down with a
//! flash. When the middle row completes, a light runs along it and sparks fly up, then it resets.
//! Drawn as 32x32 pixel art every frame (a few hundred quads), with soft light underneath.

use std::f64::consts::PI;

use eframe::egui::{
    epaint::{Mesh, Vertex},
    pos2, vec2, Color32, Painter, Pos2, Rect, Shape,
};

pub const N: usize = 32;
/// The frame shown when the hub isn't focused, and used for the app icon: the card mid-flare.
pub const REST_T: f64 = 3.3;
type Rgb = [u8; 3];

const GOLD_DARK: Rgb = [0x9a, 0x7a, 0x42];
const GOLD: Rgb = [0xce, 0xae, 0x74];
const GOLD_LIGHT: Rgb = [0xec, 0xd3, 0x9c];
const GLOW: Rgb = [0xff, 0xf3, 0xd2];
const WHITE: Rgb = [0xff, 0xff, 0xff];
const CELL: Rgb = [0x26, 0x26, 0x2c];
const CELL_TOP: Rgb = [0x34, 0x34, 0x3c];
const CELL_LOW: Rgb = [0x1b, 0x1b, 0x20];

/// One loop of the animation, in seconds.
const LOOP: f64 = 6.4;
/// Which squares get marked, and when: they finish the middle row.
const ORDER: [(usize, f64); 5] = [(7, 0.5), (3, 1.0), (1, 1.5), (5, 2.0), (4, 2.6)];
const FLARE_START: f64 = 2.75;
const FADE_START: f64 = 5.4;

fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    std::array::from_fn(|i| (a[i] as f64 + (b[i] as f64 - a[i] as f64) * t).round() as u8)
}

/// Straight (not premultiplied) RGBA, alpha 0..1.
#[derive(Clone, Copy)]
pub struct Px {
    pub rgb: [f64; 3],
    pub a: f64,
}

pub struct Sprite(pub Vec<Option<Px>>);

impl Sprite {
    fn new() -> Self {
        Sprite(vec![None; N * N])
    }
    fn put(&mut self, x: f64, y: f64, c: Rgb, a: f64) {
        let (x, y) = (x.round() as i32, y.round() as i32);
        if x < 0 || y < 0 || x >= N as i32 || y >= N as i32 || a <= 0.0 {
            return;
        }
        let a = a.min(1.0);
        let i = y as usize * N + x as usize;
        let col = c.map(|v| v as f64);
        self.0[i] = Some(match self.0[i] {
            None => Px { rgb: col, a },
            Some(cur) => {
                let na = a + cur.a * (1.0 - a);
                Px { rgb: std::array::from_fn(|k| (col[k] * a + cur.rgb[k] * cur.a * (1.0 - a)) / na), a: na }
            }
        });
    }
}

/// How strongly the completed row is lit at time `c` within the loop: 0 outside the flare.
fn flare(c: f64) -> f64 {
    if c > FLARE_START && c < FADE_START { ((c - FLARE_START) / (FADE_START - FLARE_START)).min(1.0) * PI } else { 0.0 }.sin()
}

/// The sprite at time `t` seconds.
pub fn sprite(t: f64) -> Sprite {
    let c = t.rem_euclid(LOOP);
    let fade = if c > FADE_START { (1.0 - (c - FADE_START) / 0.6).max(0.0) } else { 1.0 };
    let flare = flare(c);
    let mut s = Sprite::new();
    for i in 0..9 {
        let (gx, gy) = (4 + (i % 3) as i32 * 9, 4 + (i / 3) as i32 * 9);
        let since = ORDER.iter().find(|o| o.0 == i).map_or(-1.0, |o| c - o.1);
        let marked = since >= 0.0 && fade > 0.01;
        // A square that has just been stamped is drawn a pixel bigger, and white-hot.
        let pop = marked && since < 0.18;
        let (g0, g1) = if pop { (-1, 7) } else { (0, 6) };
        for y in g0..=g1 {
            for x in g0..=g1 {
                if (x == g0 || x == g1) && (y == g0 || y == g1) {
                    continue; // rounded corners
                }
                let mut col = if y == g0 { CELL_TOP } else if y == g1 { CELL_LOW } else { CELL };
                if marked {
                    let mut mc = if y == g0 || x == g0 { GOLD_LIGHT } else if y == g1 || x == g1 { GOLD_DARK } else { GOLD };
                    if pop {
                        mc = GLOW;
                    }
                    if (3..=5).contains(&i) && flare > 0.0 {
                        // A bright band runs along the finished row.
                        let sweep = 4.0 + (c - FLARE_START) / 1.1 * 30.0;
                        let d = ((gx + x) as f64 - sweep).abs();
                        mc = if d < 3.0 { mix(mc, WHITE, (1.0 - d / 3.0) * 0.9) } else { mix(mc, GOLD_LIGHT, flare * 0.5) };
                    }
                    col = mix(col, mc, fade);
                }
                s.put((gx + x) as f64, (gy + y) as f64, col, 1.0);
            }
        }
    }
    // The completed row throws sparks upward.
    if flare > 0.0 {
        for i in 0..8 {
            let p = ((c - FLARE_START) / 1.6 + i as f64 / 8.0).rem_euclid(1.0);
            let x = 5.0 + i as f64 * 3.0 + (p * 6.0 + i as f64).sin() * 1.2;
            s.put(x, 13.0 - p * 12.0, if p < 0.4 { GLOW } else { GOLD_LIGHT }, (p * PI).sin() * flare);
        }
    }
    s
}

/// Light behind the card while the row is lit: (centre, radius, colour, strength) in sprite pixels.
pub fn glows(t: f64) -> [((f32, f32), f32, Rgb, f32); 1] {
    [((16.0, 16.0), 17.0, [236, 200, 130], 0.4 * flare(t.rem_euclid(LOOP)) as f32)]
}

/// A soft radial light: full strength in the middle, 35% halfway out, nothing at the edge.
fn radial(painter: &Painter, c: Pos2, r: f32, rgb: Rgb, a: f32) {
    if a <= 0.0 {
        return;
    }
    let col = |a: f32| Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], (a.clamp(0.0, 1.0) * 255.0) as u8);
    let mut mesh = Mesh::default();
    let seg = 48u32;
    mesh.vertices.push(Vertex { pos: c, uv: Pos2::ZERO, color: col(a) });
    for ring in [(0.5, a * 0.35), (1.0, 0.0)] {
        for i in 0..seg {
            let ang = i as f32 / seg as f32 * std::f32::consts::TAU;
            mesh.vertices.push(Vertex { pos: c + vec2(ang.cos(), ang.sin()) * r * ring.0, uv: Pos2::ZERO, color: col(ring.1) });
        }
    }
    for i in 0..seg {
        let j = (i + 1) % seg;
        mesh.add_triangle(0, 1 + i, 1 + j);
        let (a0, a1, b0, b1) = (1 + i, 1 + j, 1 + seg + i, 1 + seg + j);
        mesh.add_triangle(a0, b0, b1);
        mesh.add_triangle(a0, b1, a1);
    }
    painter.add(Shape::mesh(mesh));
}

/// Draws the logo with its top-left at `min`, `px` points per sprite pixel.
pub fn logo(painter: &Painter, min: Pos2, px: f32, t: f64) {
    for ((x, y), r, rgb, a) in glows(t) {
        radial(painter, min + vec2(x, y) * px, r * px, rgb, a);
    }
    // One mesh of plain quads: no anti-aliasing feather, which would smear 1-point pixels away.
    // Snapped to physical pixels so they stay crisp at any display scale.
    let ppp = painter.ctx().pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let s = sprite(t);
    let mut mesh = Mesh::default();
    for y in 0..N {
        for x in 0..N {
            if let Some(p) = s.0[y * N + x] {
                let col = Color32::from_rgba_unmultiplied(p.rgb[0].round() as u8, p.rgb[1].round() as u8, p.rgb[2].round() as u8, (p.a * 255.0).round() as u8);
                let (x0, y0) = (min.x + x as f32 * px, min.y + y as f32 * px);
                mesh.add_colored_rect(Rect::from_min_max(pos2(snap(x0), snap(y0)), pos2(snap(x0 + px), snap(y0 + px))), col);
            }
        }
    }
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes the icon frame as raw RGBA (32x32, straight alpha) for tools/icon/make_icon.py:
    /// BINGOHUB_SPRITE_OUT=path cargo test --release art -- --ignored
    #[test]
    #[ignore]
    fn write_icon_sprite() {
        let path = std::env::var("BINGOHUB_SPRITE_OUT").expect("set BINGOHUB_SPRITE_OUT");
        let s = sprite(REST_T);
        let mut raw = Vec::with_capacity(N * N * 4);
        for p in &s.0 {
            match p {
                Some(p) => raw.extend(p.rgb.map(|v| v.round() as u8).into_iter().chain([(p.a * 255.0).round() as u8])),
                None => raw.extend([0, 0, 0, 0]),
            }
        }
        std::fs::write(path, raw).unwrap();
    }

    #[test]
    fn card_has_nine_squares_and_lights_the_row() {
        let s = sprite(REST_T);
        // Every square's centre is drawn.
        for i in 0..9 {
            let (x, y) = (4 + (i % 3) * 9 + 3, 4 + (i / 3) * 9 + 3);
            assert!(s.0[y * N + x].is_some(), "square {i}");
        }
        // The middle square is marked (gold, not the dark cell colour) at the rest frame.
        let mid = s.0[(4 + 9 + 3) * N + 4 + 9 + 3].unwrap();
        assert!(mid.rgb[0] > 150.0);
        assert!(glows(REST_T)[0].3 > 0.0);
    }
}
