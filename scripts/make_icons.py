#!/usr/bin/env python3
"""Generate the rplayer app icon set (teal->deep-teal gradient + gold play glyph).

Run from repo root:  python scripts/make_icons.py
Writes src-tauri/icons/{icon.png,icon.ico,icon.icns,32x32.png,128x128.png,128x128@2x.png}
"""
import os
from PIL import Image, ImageDraw, ImageFilter

S = 1024
TEAL = (55, 183, 165)      # #37B7A5
TEAL_DARK = (14, 92, 104)  # deep teal for gradient bottom
GOLD = (243, 172, 60)      # #F3AC3C

out_dir = os.path.join("src-tauri", "icons")
os.makedirs(out_dir, exist_ok=True)


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def gradient_bg(size):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    top = lerp(TEAL, (90, 210, 195), 0.0)
    bot = TEAL_DARK
    for y in range(size):
        t = y / (size - 1)
        img.paste(lerp(top, bot, t), (0, y, size, y + 1))
    # soft diagonal sheen
    sheen = Image.new("L", (size, size), 0)
    d = ImageDraw.Draw(sheen)
    d.ellipse([-size * 0.2, -size * 0.5, size * 0.9, size * 0.35], fill=60)
    sheen = sheen.filter(ImageFilter.GaussianBlur(size * 0.12))
    white = Image.new("RGBA", (size, size), (255, 255, 255, 255))
    img = Image.composite(white, img, sheen.point(lambda p: int(p * 0.5)))
    return img


def rounded_mask(size, radius_frac=0.225):
    mask = Image.new("L", (size, size), 0)
    d = ImageDraw.Draw(mask)
    r = int(size * radius_frac)
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=r, fill=255)
    return mask


def make_icon(size, ss=2):
    w = size * ss
    bg = gradient_bg(w)
    img = Image.new("RGBA", (w, w), (0, 0, 0, 0))
    img.paste(bg, (0, 0), rounded_mask(w))

    d = ImageDraw.Draw(img)
    # equalizer bars (subtle, translucent white) on the lower-left
    bars = [(0.20, 0.34), (0.28, 0.52), (0.36, 0.24)]
    for xfrac, hfrac in bars:
        x0 = w * xfrac
        bw = w * 0.05
        bh = w * hfrac
        y1 = w * 0.78
        y0 = y1 - bh
        d.rounded_rectangle([x0, y0, x0 + bw, y1], radius=int(bw * 0.4),
                            fill=(255, 255, 255, 70))

    # gold play triangle, optically centered
    cx, cy = w * 0.56, w * 0.5
    r = w * 0.20
    tri = [(cx - r * 0.72, cy - r), (cx - r * 0.72, cy + r), (cx + r * 0.98, cy)]
    # glow
    glow = Image.new("RGBA", (w, w), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    gpad = int(w * 0.03)
    gtri = [(x - gpad if i < 2 else x + gpad, y) for i, (x, y) in enumerate(tri)]
    gd.polygon(tri, fill=(243, 172, 60, 120))
    glow = glow.filter(ImageFilter.GaussianBlur(w * 0.02))
    img = Image.alpha_composite(img, glow)
    d = ImageDraw.Draw(img)
    d.polygon(tri, fill=GOLD + (255,))

    return img.resize((size, size), Image.LANCZOS)


png1024 = make_icon(1024)
png1024.save(os.path.join(out_dir, "icon.png"))
png1024.save(os.path.join(out_dir, "128x128@2x.png"))
make_icon(128).save(os.path.join(out_dir, "128x128.png"))
make_icon(32).save(os.path.join(out_dir, "32x32.png"))

# Windows .ico (multi-res). Needs up to 256.
png1024.resize((256, 256), Image.LANCZOS).save(
    os.path.join(out_dir, "icon.ico"),
    sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
)

# macOS .icns
png1024.save(os.path.join(out_dir, "icon.icns"), format="ICNS")

print("icons written to", out_dir)
for f in sorted(os.listdir(out_dir)):
    print("  ", f, os.path.getsize(os.path.join(out_dir, f)), "bytes")
