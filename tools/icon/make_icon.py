# Builds assets/icon.png and assets/hub.ico from the logo sprite in src/art.rs.
#   1. BINGOHUB_SPRITE_OUT=sprite.rgba cargo test --release art -- --ignored
#   2. python tools/icon/make_icon.py sprite.rgba
# Each size gets a dark rounded tile, the grace's soft light, then the pixels scaled by a whole
# number (or smoothly shrunk below 32 px) so they stay crisp.
import math, sys
from pathlib import Path
from PIL import Image, ImageDraw

N = 32
ROOT = Path(__file__).resolve().parents[2]
raw = Path(sys.argv[1]).read_bytes()
sprite = Image.frombytes("RGBA", (N, N), raw)

BG = (22, 22, 26)
# Same light as art::glows at the icon frame: (centre, radius, colour, strength) in sprite pixels.
REST_T = 3.3
flare = math.sin(min(1, (REST_T - 2.75) / (5.4 - 2.75)) * math.pi)
GLOWS = [((16, 16), 17, (236, 200, 130), 0.4 * flare)]


def glow_alpha(d, a):
    # Matches the mesh in art::radial: a at the centre, 35% at half radius, 0 at the edge.
    if d >= 1: return 0.0
    return a + (a * .35 - a) * (d / .5) if d < .5 else a * .35 * (1 - (d - .5) / .5)


def tile(size):
    ss = 4
    im = Image.new("RGBA", (size * ss, size * ss), (0, 0, 0, 0))
    ImageDraw.Draw(im).rounded_rectangle([0, 0, size * ss - 1, size * ss - 1], radius=size * ss * .2, fill=BG + (255,))
    im = im.resize((size, size), Image.LANCZOS)
    # Big sizes keep a margin so the figure isn't pressed against the tile edge.
    scale = (size * 7 // 8) // N if size >= 128 else size // N if size >= N else size / N
    art = N * scale
    off = (size - art) / 2
    px = im.load()
    for y in range(size):
        for x in range(size):
            r, g, b, a = px[x, y]
            if a == 0: continue
            for (cx, cy), rad, col, s in GLOWS:
                d = math.hypot((x + .5 - off) / scale - cx, (y + .5 - off) / scale - cy) / rad
                k = glow_alpha(d, s)
                r, g, b = (round(r + (col[i] - (r, g, b)[i]) * k) for i in range(3))
            px[x, y] = (r, g, b, a)
    if size >= N:
        art_img = sprite.resize((art, art), Image.NEAREST)
        im.alpha_composite(art_img, (int(off), int(off)))
    else:
        im.alpha_composite(sprite.resize((size, size), Image.LANCZOS))
    return im


sizes = [16, 24, 32, 48, 64, 128, 256]
imgs = {s: tile(s) for s in sizes}
imgs[256].save(ROOT / "assets" / "icon.png")
imgs[256].save(ROOT / "assets" / "hub.ico", format="ICO", sizes=[(s, s) for s in sizes], append_images=[imgs[s] for s in sizes if s != 256])
print("wrote assets/icon.png and assets/hub.ico")
