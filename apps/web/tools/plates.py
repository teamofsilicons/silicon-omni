#!/usr/bin/env python3
"""
Print the plates.

Every photograph and engraving on this site is quantised to one bit through the
same 8x8 Bayer matrix the hero's dither uses, so a switchboard from 1922 and a
scatter plot from this morning are printed with the same ink. Two things follow
from that and both matter more than they sound:

  a found image stops being a stock photo. One bit strips the colour cast, the
  film grain, the JPEG mush and the century, and leaves the shape. Fifteen
  images from fifteen archives come out looking like fifteen plates from one
  book;

  the dot grid has to be native to the size it is shown at. Downscaling a
  dither is how you get moire, so each size is dithered again from the
  original rather than resampled from a bigger one.

Reads plates.json, writes public/plates/<name>.png and <name>@2x.png.

    python3 tools/plates.py            # everything missing
    python3 tools/plates.py switchboard stepwell
    python3 tools/plates.py --force
"""

import json
import pathlib
import sys
import urllib.request

from PIL import Image, ImageEnhance, ImageOps

HERE = pathlib.Path(__file__).resolve().parent.parent
OUT = HERE / "public" / "plates"
CACHE = HERE / ".plate-cache"
MANIFEST = HERE / "plates.json"

# Wikimedia and several museum CDNs refuse an unnamed client, correctly.
AGENT = "omnipotent-plates/1.0 (https://github.com/teamofsilicons/omnipotent) Pillow"

WIDTHS = {"": 720, "@2x": 1440}

INK = (26, 26, 26)

BAYER = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
]


def fetch(name: str, url: str) -> pathlib.Path:
    CACHE.mkdir(exist_ok=True)
    at = CACHE / f"{name}{pathlib.Path(url.split('?')[0]).suffix or '.jpg'}"
    if at.exists() and at.stat().st_size > 20_000:
        return at
    request = urllib.request.Request(url, headers={"User-Agent": AGENT})
    with urllib.request.urlopen(request, timeout=90) as answer:
        at.write_bytes(answer.read())
    if at.stat().st_size < 20_000:
        raise SystemExit(f"{name}: {at.stat().st_size} bytes back from {url}")
    return at


def prepare(source: Image.Image, spec: dict, width: int) -> Image.Image:
    """Crop, level, and size a photograph so one bit still has something to say."""
    img = ImageOps.exif_transpose(source).convert("L")

    crop = spec.get("crop")
    if crop:
        w, h = img.size
        l, t, r, b = crop
        img = img.crop((int(w * l), int(h * t), int(w * r), int(h * b)))

    aspect = spec.get("aspect")
    if aspect:
        w, h = img.size
        want = aspect
        if w / h > want:                       # too wide: take the middle
            keep = int(h * want)
            img = img.crop(((w - keep) // 2, 0, (w - keep) // 2 + keep, h))
        else:                                  # too tall: take from the top third
            keep = int(w / want)
            top = int((h - keep) * spec.get("gravity", 0.35))
            img = img.crop((0, top, w, top + keep))

    # An archive scan is usually flat and a little grey. One bit has no room for
    # subtlety, so the tones are pulled apart before they are thrown away.
    img = ImageOps.autocontrast(img, cutoff=spec.get("cutoff", 1.5))
    img = ImageEnhance.Contrast(img).enhance(spec.get("contrast", 1.16))
    if spec.get("invert"):
        img = ImageOps.invert(img)

    gamma = spec.get("gamma", 1.0)
    if gamma != 1.0:
        table = [min(255, int(255 * ((i / 255) ** (1 / gamma)))) for i in range(256)]
        img = img.point(table)

    height = max(1, round(width * img.size[1] / img.size[0]))
    return img.resize((width, height), Image.LANCZOS)


def dither(grey: Image.Image, spread: float) -> Image.Image:
    """
    Ordered, not diffused.

    Floyd-Steinberg looks better on a photograph and wrong on a page like this:
    error diffusion makes an organic, slightly noisy grain, and an ordered
    matrix makes a regular screen — the same regularity as a halftone, a
    newspaper, and the field of dots at the top of the page. The site is a
    printed thing, so the plates are screened rather than stippled.
    """
    w, h = grey.size
    src = grey.load()
    out = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    dst = out.load()
    for y in range(h):
        row = BAYER[y & 7]
        for x in range(w):
            # bias the threshold outward from the middle so blacks stay black
            # and the paper stays paper instead of everything going 50% grey
            threshold = 128 + (row[x & 7] - 31.5) * spread
            if src[x, y] < threshold:
                dst[x, y] = (*INK, 255)
    return out


def print_plate(name: str, spec: dict, force: bool) -> bool:
    """Print both sizes. Returns True if plates.json needs writing back."""
    OUT.mkdir(parents=True, exist_ok=True)
    wanted = [(suffix, OUT / f"{name}{suffix}.png") for suffix in WIDTHS]
    if not force and all(at.exists() for _, at in wanted) and spec.get("size"):
        print(f"  {name}: already printed  {spec['size'][0]}x{spec['size'][1]}")
        return False
    origin = fetch(name, spec["url"])
    with Image.open(origin) as source:
        source.load()
        for suffix, at in wanted:
            grey = prepare(source, spec, WIDTHS[suffix])
            dither(grey, spec.get("spread", 2.0)).save(at, optimize=True)
            size = at.stat().st_size // 1024
            print(f"  {name}{suffix}: {grey.size[0]}x{grey.size[1]}  {size} KB")
            if suffix == "":
                # written back so <img> can carry width and height and the page
                # never reflows around a plate that has not loaded yet
                spec["size"] = list(grey.size)
    return True


def main() -> None:
    argv = [a for a in sys.argv[1:] if not a.startswith("-")]
    force = "--force" in sys.argv
    book = json.loads(MANIFEST.read_text())
    plates = book["plates"]
    names = argv or list(plates)
    bad = []
    dirty = False
    for name in names:
        spec = plates.get(name)
        if not spec:
            print(f"  {name}: not in plates.json")
            continue
        try:
            dirty |= print_plate(name, spec, force)
        except Exception as why:                       # a dead archive link is news, not a crash
            bad.append((name, why))
            print(f"  {name}: FAILED — {why}")
    if dirty:
        MANIFEST.write_text(json.dumps(book, indent=2, ensure_ascii=False) + "\n")
    if bad:
        print(f"\n{len(bad)} plate(s) did not print:")
        for name, why in bad:
            print(f"  {name}  {why}")
        sys.exit(1)


if __name__ == "__main__":
    main()
