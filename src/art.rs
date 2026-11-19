//! The logo: a hooded Tarnished kneeling with their sword planted, lit from behind by a grace.
//! Drawn as 32x32 pixel art every frame (a few hundred rects), with soft light underneath.

use std::f64::consts::PI;

use eframe::egui::{
    epaint::{Mesh, Vertex},
    pos2, vec2, Color32, Painter, Pos2, Rect, Shape,
};

pub const N: usize = 32;
type Rgb = [u8; 3];

const GOLD_DEEP: Rgb = [0x6e, 0x51, 0x26];
const GOLD_DARK: Rgb = [0x9a, 0x7a, 0x42];
const GOLD: Rgb = [0xce, 0xae, 0x74];
const GLOW: Rgb = [0xff, 0xf3, 0xd2];

/// Left half of the hood, mirrored. H outline, h hood, f the shadow where a face would be.
const HOOD: [&str; 8] = [
    "...............H",
    "..............Hh",
    ".............Hhh",
    ".............Hhf",
    "............Hhff",
    "............Hhff",
    "............Hhff",
    "............HHhf",
];

fn key(ch: u8) -> Rgb {
    match ch {
        b'H' => [0x14, 0x14, 0x18],
        b'h' => [0x2a, 0x2a, 0x32],
        b'f' => [0x08, 0x08, 0x0a],
        b'C' => [0x1e, 0x1e, 0x24],
        b'c' => [0x30, 0x30, 0x3a],
        b'l' => [0x43, 0x43, 0x4f],
        b'G' => GOLD,
        b'g' => [0x55, 0x55, 0x5f],
        b'S' => [0xd8, 0xd8, 0xe0],
        _ => [0x8a, 0x8a, 0x94], // s
    }
}

fn hash(x: i32, y: i32, z: i32) -> f64 {
    let h = x.wrapping_mul(374_761_393).wrapping_add(y.wrapping_mul(668_265_263)).wrapping_add(z.wrapping_mul(2_147_483_647)) as u32;
    let h = ((h ^ (h >> 13)) as i32).wrapping_mul(1_274_126_177) as u32;
    (h ^ (h >> 16)) as f64 / 4_294_967_296.0
}

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
    pub fn get(&self, x: i32, y: i32) -> Option<Px> {
        if x < 0 || y < 0 || x >= N as i32 || y >= N as i32 { None } else { self.0[y as usize * N + x as usize] }
    }
}

/// How bright the grace is: a slow breath with a little flicker.
fn pulse(t: f64) -> f64 {
    0.78 + 0.22 * (t * 2.4).sin() + 0.06 * (t * 7.3).sin()
}

/// The sprite at time `t` seconds.
pub fn sprite(t: f64) -> Sprite {
    let pulse = pulse(t);
    let mut fig = Sprite::new();
    for (i, row) in HOOD.iter().enumerate() {
        for (x, ch) in row.bytes().enumerate() {
            if ch != b'.' {
                fig.put(x as f64, i as f64 + 2.0, key(ch), 1.0);
                fig.put(31.0 - x as f64, i as f64 + 2.0, key(ch), 1.0);
            }
        }
    }
    // Cloak: rounded shoulders, widening to the ground, darker towards the hem.
    for y in 10..=27i32 {
        let hw = if y < 14 { [3.5, 5.5, 7.0, 8.0][(y - 10) as usize] } else { 8.4 + (y - 14) as f64 * 0.22 };
        let (lo, hi) = ((16.0 - hw).round() as i32, (15.0 + hw).round() as i32);
        for x in lo..=hi {
            let dx = (x as f64 - 15.5).abs();
            let ch = if x == lo || x == hi {
                b'H'
            } else if y < 12 && dx < 5.0 {
                b'h'
            } else if y > 22 || (y > 17 && (dx == 3.5 || dx == 7.5) && hash(x, y, 0) > 0.25) {
                b'C'
            } else if y > 12 && y < 18 && dx > 5.0 && dx < 7.0 {
                b'l'
            } else {
                b'c'
            };
            fig.put(x as f64, y as f64, key(ch), 1.0);
        }
    }
    // The hem stirs.
    let flap = (t * 4.0).floor() as i32;
    for x in 6..=25 {
        if hash(x, flap, 0) > 0.45 {
            fig.put(x as f64, 28.0, key(b'C'), 1.0);
        }
    }
    // Sword planted in front, hands on the pommel.
    fig.put(15.0, 12.0, GOLD, 1.0);
    fig.put(16.0, 12.0, GOLD, 1.0);
    for y in 13..=14 {
        for x in 14..=17 {
            fig.put(x as f64, y as f64, key(b'g'), 1.0);
        }
    }
    for x in 12..=19 {
        fig.put(x as f64, 15.0, if x == 12 || x == 19 { GOLD_DARK } else { GOLD }, 1.0);
    }
    for y in 16..=27 {
        fig.put(15.0, y as f64, key(b'S'), 1.0);
        fig.put(16.0, y as f64, key(b's'), 1.0);
    }
    fig.put(15.0, 28.0, key(b's'), 1.0);

    // Rim light: every edge facing out catches the gold.
    let mut out = Sprite::new();
    for y in 0..N as i32 {
        for x in 0..N as i32 {
            let Some(p) = fig.get(x, y) else { continue };
            let mut col: Rgb = p.rgb.map(|v| v.round() as u8);
            let edge = fig.get(x - 1, y).is_none() || fig.get(x + 1, y).is_none() || fig.get(x, y - 1).is_none();
            if edge && y < 28 {
                col = mix(col, GOLD, 0.62 * pulse);
            }
            out.put(x as f64, y as f64, col, 1.0);
        }
    }
    // Ground, and embers rising past.
    for x in 4..=27 {
        out.put(x as f64, 29.0, GOLD_DEEP, 0.55 * (1.0 - (x as f64 - 15.5).abs() / 13.0));
    }
    for i in 0..6 {
        let p = (t / 3.2 + i as f64 / 6.0).rem_euclid(1.0);
        let side = if i % 2 == 1 { 1.0 } else { -1.0 };
        let x = 16.0 + side * (5.0 + (i % 3) as f64 * 3.0) + (p * 8.0 + i as f64).sin() * 1.4;
        out.put(x, 24.0 - p * 22.0, GLOW, (p * PI).sin() * 0.9);
    }
    out
}

/// The grace's light behind the figure: (centre, radius, colour, strength) in sprite pixels.
pub fn glows(t: f64) -> [((f32, f32), f32, Rgb, f32); 2] {
    let p = pulse(t) as f32;
    [((16.0, 13.0), 17.0, [226, 186, 112], 0.55 * p), ((16.0, 10.0), 7.0, [255, 236, 190], 0.45 * p)]
}

/// A soft radial light: full strength in the middle, 35% halfway out, nothing at the edge.
fn radial(painter: &Painter, c: Pos2, r: f32, rgb: Rgb, a: f32) {
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
        let s = sprite(0.6);
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
    fn sprite_has_a_figure() {
        let s = sprite(1.0);
        assert!(s.0.iter().filter(|p| p.is_some()).count() > 300);
        assert!(s.get(15, 20).is_some(), "the blade is drawn");
    }
}
