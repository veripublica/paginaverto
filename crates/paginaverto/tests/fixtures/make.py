"""Writes the image-codec fixtures from a fixed pseudo-random pattern, which the
tests rebuild to compare pixel by pixel. Nothing here is from a document. The
pattern is 16-pixel blocks, so that it compresses, on a 400-pixel square: a gray
figure smaller than that on a side is not kept.

    uv run --no-project --with pillow==13.0.0 python make.py

pattern.g4    CCITT Group 4 data of the bilevel pattern, 400 x 400, 0 = black.
pattern.jp2   the gray pattern, 400 x 400, JPEG 2000, lossless.
pattern-rgb.jp2  the RGB pattern, 400 x 400, JPEG 2000, lossless.
"""

import io
import pathlib

from PIL import Image, TiffImagePlugin

W, H = 400, 400
B = 16
COLS = (W + B - 1) // B
HERE = pathlib.Path(__file__).parent


def lcg(n):
    """The C library's rand(): x = x * 1103515245 + 12345 mod 2^31, from 1."""
    x = 1
    out = []
    for _ in range(n):
        x = (x * 1103515245 + 12345) % (1 << 31)
        out.append(x >> 16)
    return out


def main():
    n = COLS * ((H + B - 1) // B)
    r = lcg(n * 3)
    block = [(y // B) * COLS + x // B for y in range(H) for x in range(W)]
    bilevel = Image.new("1", (W, H))
    bilevel.putdata([255 if r[b] & 1 else 0 for b in block])
    gray = Image.new("L", (W, H))
    gray.putdata([r[b] & 255 for b in block])
    rgb = Image.new("RGB", (W, H))
    rgb.putdata([(r[b] & 255, r[b + n] & 255, r[b + 2 * n] & 255) for b in block])

    # Group 4 in a one-strip TIFF; the strip is the raw CCITT data a PDF holds.
    buf = io.BytesIO()
    TiffImagePlugin.WRITE_LIBTIFF = True
    bilevel.save(buf, "TIFF", compression="group4", rowsperstrip=H)
    tiff = Image.open(io.BytesIO(buf.getvalue()))
    offset = tiff.tag_v2[273][0] if isinstance(tiff.tag_v2[273], tuple) else tiff.tag_v2[273]
    length = tiff.tag_v2[279][0] if isinstance(tiff.tag_v2[279], tuple) else tiff.tag_v2[279]
    print("photometric", tiff.tag_v2.get(262))
    (HERE / "pattern.g4").write_bytes(buf.getvalue()[offset : offset + length])

    gray.save(HERE / "pattern.jp2", "JPEG2000", irreversible=False)
    rgb.save(HERE / "pattern-rgb.jp2", "JPEG2000", irreversible=False)


if __name__ == "__main__":
    main()
