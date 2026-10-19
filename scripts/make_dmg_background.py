"""Draws the DMG installer window background (the drag-to-Applications screen).

Run: python3 scripts/make_dmg_background.py
Writes src-tauri/icons/dmg-background.png at 2x with 144 DPI, so Finder shows it
crisp on Retina at 640 x 400 points. Icon spots must match tauri.conf.json:
app at (170, 250), Applications at (470, 250).
"""
import io
import math
from pathlib import Path

from fontTools.ttLib import TTFont
from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "src-tauri" / "icons" / "dmg-background.png"
WOFF2 = ROOT / "node_modules/@fontsource-variable/sora/files/sora-latin-wght-normal.woff2"

W, H = 640, 400          # window content, in points
SCALE = 2                # output pixels per point (Retina)
SS = 3                   # extra supersampling for smooth curves
K = SCALE * SS           # drawing units per point

CREAM = (250, 246, 236)
CREAM_DEEP = (243, 236, 220)
BUTTER_HI = (248, 230, 176)
BUTTER = (239, 210, 127)
BUTTER_DEEP = (214, 168, 74)
INK = (46, 40, 32)
MUTED = (107, 97, 82)

APP = (170, 250)
APPS = (470, 250)


def font(size_pt, weight):
    tt = TTFont(str(WOFF2))
    tt.flavor = None
    buf = io.BytesIO()
    tt.save(buf)
    buf.seek(0)
    f = ImageFont.truetype(buf, int(size_pt * K))
    f.set_variation_by_axes([weight])
    return f


def vgradient(w, h, top, bottom):
    g = Image.new("RGB", (1, h))
    for y in range(h):
        t = y / max(1, h - 1)
        g.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(top, bottom)))
    return g.resize((w, h))


def butter_fill(size):
    """Butter colour with a soft highlight band near the top."""
    w, h = size
    g = Image.new("RGB", (1, h))
    for y in range(h):
        t = y / max(1, h - 1)
        if t < 0.18:
            c = [a + (b - a) * (t / 0.18) for a, b in zip(BUTTER_HI, BUTTER)]
        else:
            c = [a + (b - a) * min(1, (t - 0.18) / 0.82) * 0.35 for a, b in zip(BUTTER, BUTTER_DEEP)]
        g.putpixel((0, y), tuple(round(v) for v in c))
    return g.resize((w, h))


def drips_mask():
    """The melting edge along the top: a band with rounded drips of varied length."""
    m = Image.new("L", (W * K, H * K), 0)
    d = ImageDraw.Draw(m)
    band = 30
    d.rectangle([0, 0, W * K, band * K], fill=255)
    # (x centre, width, length below the band), in points. Hand-placed so it reads as melting, not a pattern.
    drips = [(38, 26, 22), (92, 18, 44), (140, 30, 16), (205, 22, 62), (262, 34, 26), (318, 20, 40),
             (372, 28, 18), (430, 22, 70), (486, 30, 24), (540, 18, 46), (596, 28, 20)]
    for x, w, length in drips:
        r = w / 2
        top = band - 6
        bottom = band + length
        d.rectangle([(x - r) * K, top * K, (x + r) * K, (bottom - r) * K], fill=255)
        d.ellipse([(x - r) * K, (bottom - 2 * r) * K, (x + r) * K, bottom * K], fill=255)
        # soft fillets where the drip meets the band
        f = 7
        for side in (-1, 1):
            cx = x + side * (r + f)
            d.rectangle([min(cx, x + side * r) * K, band * K, max(cx, x + side * r) * K, (band + f) * K], fill=255)
            d.ellipse([(cx - f) * K, band * K, (cx + f) * K, (band + 2 * f) * K], fill=0)
    # a couple of falling drops
    for x, y, r in [(205, 84, 4.5), (430, 94, 3.5)]:
        drop(d, x, y, r)
    return m


def drop(d, x, y, r):
    """Teardrop: circle with a point on top."""
    d.ellipse([(x - r) * K, (y - r) * K, (x + r) * K, (y + r) * K], fill=255)
    d.polygon([((x - r * 0.82) * K, (y - r * 0.5) * K), (x * K, (y - r * 2.3) * K), ((x + r * 0.82) * K, (y - r * 0.5) * K)], fill=255)


def arc_points(p0, p1, lift, n=80):
    (x0, y0), (x1, y1) = p0, p1
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2 - lift
    pts = []
    for i in range(n + 1):
        t = i / n
        x = (1 - t) ** 2 * x0 + 2 * (1 - t) * t * cx + t * t * x1
        y = (1 - t) ** 2 * y0 + 2 * (1 - t) * t * cy + t * t * y1
        pts.append((x, y))
    return pts


def smear_arrow_mask():
    """A swipe of butter from the app to Applications: tapers in, swells, ends in a soft arrowhead."""
    m = Image.new("L", (W * K, H * K), 0)
    d = ImageDraw.Draw(m)
    start, end = (APP[0] + 76, APP[1] - 6), (APPS[0] - 84, APPS[1] - 6)
    pts = arc_points(start, end, 30)
    for i, (x, y) in enumerate(pts):
        t = i / (len(pts) - 1)
        width = 3 + 6 * math.sin(math.pi * min(1, t * 1.15)) ** 0.8  # thin start, full body
        d.ellipse([(x - width) * K, (y - width) * K, (x + width) * K, (y + width) * K], fill=255)
    # arrowhead: two rounded strokes
    (ex, ey), (px, py) = pts[-1], pts[-6]
    ang = math.atan2(ey - py, ex - px)
    for side in (-1, 1):
        a = ang + math.pi + side * 0.62
        for j in range(30):
            s = j / 29 * 20
            x, y = ex + math.cos(a) * s, ey + math.sin(a) * s
            r = 4.2
            d.ellipse([(x - r) * K, (y - r) * K, (x + r) * K, (y + r) * K], fill=255)
    return m


def soft_shadow(mask, offset, blur, opacity):
    sh = mask.filter(ImageFilter.GaussianBlur(blur * K))
    sh = sh.point(lambda v: int(v * opacity))
    out = Image.new("L", mask.size, 0)
    out.paste(sh, (int(offset[0] * K), int(offset[1] * K)))
    return out


def main():
    img = vgradient(W * K, H * K, CREAM, CREAM_DEEP).convert("RGBA")

    # melting top edge, with a warm shadow so it sits on the cream
    dm = drips_mask()
    img.paste((190, 150, 70, 255), (0, 0), soft_shadow(dm, (0, 3), 5, 0.28))
    img.paste(butter_fill(img.size), (0, 0), dm)
    # the butter swipe arrow
    am = smear_arrow_mask()
    img.paste((190, 150, 70, 255), (0, 0), soft_shadow(am, (0, 2), 3, 0.25))
    img.paste(butter_fill(img.size), (0, 0), am)

    # words
    d = ImageDraw.Draw(img)
    head = font(26, 650)
    sub = font(12.5, 450)
    d.text((W * K / 2, 132 * K), "Spread it on.", font=head, fill=INK, anchor="mm")
    d.text((W * K / 2, 160 * K), "Drag Butters into Applications", font=sub, fill=MUTED, anchor="mm")

    out = img.convert("RGB").resize((W * SCALE, H * SCALE), Image.LANCZOS)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    out.save(OUT, dpi=(72 * SCALE, 72 * SCALE), optimize=True)
    print("wrote", OUT, out.size)


if __name__ == "__main__":
    main()
