"""Draws Butters' icons: a soft, melting pat of butter on a cream tile.

Run: python3 scripts/make_icons.py
Writes src-tauri/icons/app-icon.png (feed to `cargo tauri icon`) and the
menu bar / tray icons (colour, not template, so they stay butter-coloured).
"""
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"

CREAM_TOP = (255, 251, 240)
CREAM_BOTTOM = (245, 234, 208)
BUTTER_TOP = (255, 238, 178)
BUTTER_FRONT = (244, 206, 106)
BUTTER_SIDE = (226, 178, 74)
BUTTER_PUDDLE = (240, 200, 100)
EDGE = (176, 128, 40)
SHADOW = (150, 110, 40)


def vgrad(size, top, bottom):
    w, h = size
    g = Image.new("RGB", (1, h))
    for y in range(h):
        t = y / max(1, h - 1)
        g.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(top, bottom)))
    return g.resize((w, h))


def soften(mask, r):
    """Rounds every corner of a hard mask: blur, then re-threshold at the midpoint."""
    return mask.filter(ImageFilter.GaussianBlur(r)).point(lambda v: 255 if v > 127 else 0).filter(
        ImageFilter.GaussianBlur(max(1, r / 6))
    )


def poly_mask(size, pts):
    m = Image.new("L", size, 0)
    ImageDraw.Draw(m).polygon(pts, fill=255)
    return m


def pat_layers(s, ox=0.0, oy=0.0, k=1.0, melt=0.0):
    """Masks for a butter pat in an s x s box: top, front, side faces plus puddle and drips.
    Coordinates are fractions of s, scaled by k around the centre and offset by (ox, oy)."""

    def P(x, y):
        cx, cy = 0.5, 0.52
        return ((cx + (x - cx) * k + ox) * s, (cy + (y - cy) * k + oy) * s)

    size = (s, s)
    top = poly_mask(size, [P(0.20, 0.40), P(0.42, 0.27), P(0.82, 0.27), P(0.60, 0.40)])
    front = poly_mask(size, [P(0.20, 0.40), P(0.60, 0.40), P(0.60, 0.62), P(0.20, 0.62)])
    side = poly_mask(size, [P(0.60, 0.40), P(0.82, 0.27), P(0.82, 0.49), P(0.60, 0.62)])
    # Melt: drips down the front face and a puddle spreading underneath.
    drips = Image.new("L", size, 0)
    d = ImageDraw.Draw(drips)
    drips_spec = [(0.29, 0.065, 0.13), (0.45, 0.05, 0.075)]
    if melt:
        # melting: longer drips, a third one, and the pat runs into a wider puddle
        drips_spec = [(0.27, 0.075, 0.13 + 0.12 * melt), (0.44, 0.06, 0.075 + 0.1 * melt), (0.55, 0.05, 0.05 + 0.08 * melt)]
    for x, w, length in drips_spec:
        (x0, y0), (x1, y1) = P(x - w / 2, 0.58), P(x + w / 2, 0.62 + length)
        d.rounded_rectangle([x0, y0, x1, y1], radius=(x1 - x0) / 2, fill=255)
    puddle = Image.new("L", size, 0)
    spread = 0.06 * melt
    (px0, py0), (px1, py1) = P(0.13 - spread, 0.585), P(0.90 + spread, 0.74 + spread * 0.6)
    ImageDraw.Draw(puddle).ellipse([px0, py0, px1, py1], fill=255)
    return top, front, side, drips, puddle


BROWNED = dict(top=(246, 206, 120), front=(222, 154, 62), side=(190, 118, 40), puddle=(214, 140, 56))


def draw_pat(base, s, ox=0.0, oy=0.0, k=1.0, round_r=None, outline=0.0, shine=True, melt=0.0, palette=None):
    top, front, side, drips, puddle = pat_layers(s, ox, oy, k, melt)
    pal = palette or {}
    r = round_r if round_r is not None else s * 0.018
    body = soften(ImageChops.lighter(ImageChops.lighter(top, front), side), r)
    melt_mask = soften(ImageChops.lighter(drips, puddle), r * 0.8)
    whole = ImageChops.lighter(body, melt_mask)

    # Soft contact shadow under the puddle.
    sh = whole.filter(ImageFilter.GaussianBlur(s * 0.03)).point(lambda v: int(v * 0.32))
    shadow = Image.new("L", base.size, 0)
    shadow.paste(sh, (0, int(s * 0.02)))
    base.paste(SHADOW, (0, 0), shadow)

    if outline:
        ring = whole.filter(ImageFilter.MaxFilter(int(outline) * 2 + 1))
        base.paste(EDGE, (0, 0), ring)

    front_c = pal.get("front", BUTTER_FRONT)
    base.paste(pal.get("puddle", BUTTER_PUDDLE), (0, 0), melt_mask)
    base.paste(vgrad(base.size, front_c, pal.get("side", BUTTER_SIDE)), (0, 0), ImageChops.multiply(side, body))
    base.paste(vgrad(base.size, pal.get("top", (250, 220, 130)), front_c), (0, 0), ImageChops.multiply(front, body))
    base.paste(vgrad(base.size, pal.get("top", BUTTER_TOP), pal.get("top", (250, 224, 146))), (0, 0), ImageChops.multiply(top, body))
    base.paste(vgrad(base.size, pal.get("top", (250, 220, 130)), front_c), (0, 0), soften(drips, r * 0.8))

    if shine:
        # A soft highlight on the top face and a gloss on the puddle.
        g = Image.new("L", base.size, 0)
        d = ImageDraw.Draw(g)

        def P(x, y):
            cx, cy = 0.5, 0.52
            return ((cx + (x - cx) * k + ox) * s, (cy + (y - cy) * k + oy) * s)

        d.polygon([P(0.36, 0.33), P(0.46, 0.295), P(0.66, 0.295), P(0.56, 0.33)], fill=150)
        (gx0, gy0), (gx1, gy1) = P(0.66, 0.665), P(0.80, 0.69)
        d.ellipse([gx0, gy0, gx1, gy1], fill=120)
        base.paste((255, 253, 240), (0, 0), g.filter(ImageFilter.GaussianBlur(s * 0.008)))


def app_icon(path, size=1024):
    S = size * 2
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    # macOS icon grid: 824/1024 body, ~185/1024 corner radius, soft drop shadow.
    m = (S - S * 824 / 1024) / 2
    tile = Image.new("L", (S, S), 0)
    ImageDraw.Draw(tile).rounded_rectangle([m, m, S - m, S - m], radius=S * 185 / 1024, fill=255)
    shadow = tile.filter(ImageFilter.GaussianBlur(S * 0.018)).point(lambda v: int(v * 0.35))
    sh = Image.new("L", (S, S), 0)
    sh.paste(shadow, (0, int(S * 0.012)))
    img.paste((90, 64, 20, 255), (0, 0), sh)
    img.paste(vgrad((S, S), CREAM_TOP, CREAM_BOTTOM), (0, 0), tile)

    art = Image.new("RGB", (S, S), CREAM_TOP)
    art.paste(vgrad((S, S), CREAM_TOP, CREAM_BOTTOM), (0, 0))
    draw_pat(art, S, k=0.86, oy=0.0)
    img.paste(art, (0, 0), tile)
    # Hairline edge so the tile holds its shape on light backgrounds.
    edge = ImageChops.subtract(tile, tile.filter(ImageFilter.MinFilter(5)))
    img.paste((210, 186, 140, 255), (0, 0), edge.point(lambda v: int(v * 0.5)))
    img.resize((size, size), Image.LANCZOS).save(path)


def tray_icon(path, px=44, alert=False):
    """Menu bar / tray: the pat in colour. Under strain it's a melting,
    browning pat with longer drips (no alarm dot); static, so it costs nothing."""
    S = px * 8
    melt = 1.0 if alert else 0.0
    base = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    canvas = Image.new("RGB", (S, S), (0, 0, 0))
    k, oy = (1.12, -0.01) if alert else (1.18, 0.02)
    top, front, side, drips, puddle = pat_layers(S, 0, oy, k, melt)
    r = S * 0.03
    body = soften(ImageChops.lighter(ImageChops.lighter(top, front), side), r)
    melt_mask = soften(ImageChops.lighter(drips, puddle), r * 0.8)
    whole = ImageChops.lighter(body, melt_mask)
    ring = whole.filter(ImageFilter.MaxFilter(int(S * 0.035) * 2 + 1))
    canvas.paste((150, 86, 24) if alert else EDGE, (0, 0), ring)
    draw_pat(canvas, S, oy=oy, k=k, round_r=r, shine=False, melt=melt, palette=BROWNED if alert else None)
    base.paste(canvas, (0, 0), ring)
    base.resize((px, px), Image.LANCZOS).save(path)


OUT.mkdir(parents=True, exist_ok=True)
tray_icon(OUT / "tray.png")
tray_icon(OUT / "tray-alert.png", alert=True)
app_icon(OUT / "app-icon.png")
print("icons written to", OUT)
