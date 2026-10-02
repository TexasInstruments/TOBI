#!/usr/bin/env python3
"""Recolor TI's monochrome 32-bit BMP logo for TOBI's dark boot theme."""

import gzip
import struct
import sys
from pathlib import Path


TI_RED = (0xC8, 0x20, 0x2F)


def recolor(source: Path, destination: Path) -> None:
    bitmap = bytearray(source.read_bytes())

    if bitmap[:2] != b"BM" or len(bitmap) < 54:
        raise ValueError(f"{source} is not a BMP file")

    pixel_offset = struct.unpack_from("<I", bitmap, 10)[0]
    width, height = struct.unpack_from("<ii", bitmap, 18)
    planes, bits_per_pixel = struct.unpack_from("<HH", bitmap, 26)
    compression = struct.unpack_from("<I", bitmap, 30)[0]

    if width <= 0 or height == 0 or planes != 1 or bits_per_pixel != 32:
        raise ValueError("expected a non-empty 32-bit BMP")
    if compression not in (0, 3):
        raise ValueError("expected an uncompressed or bitfield 32-bit BMP")

    pixel_count = width * abs(height)
    pixel_end = pixel_offset + pixel_count * 4
    if pixel_end > len(bitmap):
        raise ValueError("BMP pixel data is truncated")

    red, green, blue = TI_RED
    for offset in range(pixel_offset, pixel_end, 4):
        source_blue, source_green, source_red = bitmap[offset : offset + 3]
        intensity = (source_red + source_green + source_blue) // 3
        bitmap[offset] = blue * intensity // 255
        bitmap[offset + 1] = green * intensity // 255
        bitmap[offset + 2] = red * intensity // 255

    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("wb") as output:
        with gzip.GzipFile(filename="", mode="wb", fileobj=output, mtime=0) as archive:
            archive.write(bitmap)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit(f"usage: {sys.argv[0]} INPUT.bmp OUTPUT.bmp.gz")
    recolor(Path(sys.argv[1]), Path(sys.argv[2]))
