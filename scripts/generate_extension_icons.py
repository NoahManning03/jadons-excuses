#!/usr/bin/env python3
"""Generate JE assets: Chrome extension, macOS tray templates, macOS Dock / bundle icons."""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
ICONS_DIR = ROOT / "browser-extension" / "icons"
STORE_DIR = ROOT / "browser-extension" / "store-assets"

TANGERINE = (242, 133, 0, 255)
BLACK = (0, 0, 0, 255)
WHITE = (255, 255, 255, 255)

TRAY_DIR = ROOT / "src-tauri" / "icons" / "tray"
TAURI_ICONS = ROOT / "src-tauri" / "icons"

# Dock / .app: tangerine rounded plate + white JE (full-color, not template).
_DOCK_CORNER_RATIO = 0.225
_DOCK_FONT_RATIO = 0.50
_DOCK_KERN_RATIO = 0.055

# Prefer bold sans; fall back across macOS / Linux / common installs.
FONT_CANDIDATES = [
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
    "/Library/Fonts/Arial Bold.ttf",
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
]


def load_font(size_px: int) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    for fp in FONT_CANDIDATES:
        p = Path(fp)
        if not p.is_file():
            continue
        try:
            return ImageFont.truetype(str(p), size_px)
        except OSError:
            continue
    return ImageFont.load_default()


def draw_je_monogram(
    size: int,
    font_ratio: float,
    kern_ratio: float,
    fill: tuple[int, int, int, int],
) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    font_px = max(6, int(size * font_ratio))
    font = load_font(font_px)

    j, e = "J", "E"
    tb_j = draw.textbbox((0, 0), j, font=font)
    tb_e = draw.textbbox((0, 0), e, font=font)
    wj = tb_j[2] - tb_j[0]
    we = tb_e[2] - tb_e[0]
    hj = tb_j[3] - tb_j[1]
    he = tb_e[3] - tb_e[1]
    kerning = max(0, int(size * kern_ratio))
    tw = wj + we - kerning
    x0 = (size - tw) / 2
    y_top = (size - max(hj, he)) / 2

    draw.text((x0 - tb_j[0], y_top - tb_j[1]), j, font=font, fill=fill)
    draw.text((x0 + wj - kerning - tb_e[0], y_top - tb_e[1]), e, font=font, fill=fill)
    return img


def draw_macos_dock_icon(size: int) -> Image.Image:
    """Tangerine rounded square (~squircle-like radius) with centered white JE."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    radius = max(2, int(round(size * _DOCK_CORNER_RATIO)))
    # Inclusive box through bottom-right pixel.
    draw.rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=TANGERINE)
    je = draw_je_monogram(size, _DOCK_FONT_RATIO, _DOCK_KERN_RATIO, WHITE)
    return Image.alpha_composite(img, je)


def write_macos_bundle_icons() -> None:
    """PNG ladder + icon.png + Tauri fallbacks + icon.icns (macOS iconutil)."""
    TAURI_ICONS.mkdir(parents=True, exist_ok=True)
    sizes = (16, 32, 64, 128, 256, 512, 1024)
    for s in sizes:
        out = TAURI_ICONS / f"icon-{s}.png"
        draw_macos_dock_icon(s).save(out, format="PNG")
        print(f"Wrote {out}")

    master = TAURI_ICONS / "icon.png"
    shutil.copyfile(TAURI_ICONS / "icon-1024.png", master)
    print(f"Wrote {master} (1024 master)")

    shutil.copyfile(TAURI_ICONS / "icon-32.png", TAURI_ICONS / "32x32.png")
    shutil.copyfile(TAURI_ICONS / "icon-128.png", TAURI_ICONS / "128x128.png")
    shutil.copyfile(TAURI_ICONS / "icon-256.png", TAURI_ICONS / "128x128@2x.png")
    print(f"Updated {TAURI_ICONS / '32x32.png'}")
    print(f"Updated {TAURI_ICONS / '128x128.png'}")
    print(f"Updated {TAURI_ICONS / '128x128@2x.png'}")

    iconset = TAURI_ICONS / "icon.iconset"
    if iconset.exists():
        shutil.rmtree(iconset)
    iconset.mkdir(parents=True)

    pairs = [
        ("icon-16.png", "icon_16x16.png"),
        ("icon-32.png", "icon_16x16@2x.png"),
        ("icon-32.png", "icon_32x32.png"),
        ("icon-64.png", "icon_32x32@2x.png"),
        ("icon-128.png", "icon_128x128.png"),
        ("icon-256.png", "icon_128x128@2x.png"),
        ("icon-256.png", "icon_256x256.png"),
        ("icon-512.png", "icon_256x256@2x.png"),
        ("icon-512.png", "icon_512x512.png"),
        ("icon-1024.png", "icon_512x512@2x.png"),
    ]
    for src, dst in pairs:
        shutil.copyfile(TAURI_ICONS / src, iconset / dst)

    icns_out = TAURI_ICONS / "icon.icns"
    try:
        subprocess.run(
            [
                "iconutil",
                "-c",
                "icns",
                str(iconset),
                "-o",
                str(icns_out),
            ],
            check=True,
            capture_output=True,
            text=True,
        )
        print(f"Wrote {icns_out}")
    except FileNotFoundError:
        print(
            "iconutil not found (not macOS?): skipped .icns — install PNGs are still valid.",
            file=sys.stderr,
        )
    except subprocess.CalledProcessError as e:
        print(e.stderr or e.stdout or str(e), file=sys.stderr)
        print("iconutil failed; PNG assets still written.", file=sys.stderr)
    finally:
        shutil.rmtree(iconset, ignore_errors=True)


def main() -> None:
    ICONS_DIR.mkdir(parents=True, exist_ok=True)
    STORE_DIR.mkdir(parents=True, exist_ok=True)

    specs = [
        (16, 0.78, 0.09),  # heavier relative size + tight kern for toolbar legibility
        (48, 0.72, 0.06),
        (128, 0.72, 0.06),
    ]

    for s, fr, kr in specs:
        out = ICONS_DIR / f"icon-{s}.png"
        draw_je_monogram(s, fr, kr, TANGERINE).save(out, format="PNG")
        print(f"Wrote {out}")

    marketing = STORE_DIR / "icon-marketing.png"
    draw_je_monogram(128, 0.72, 0.06, TANGERINE).save(marketing, format="PNG")
    print(f"Wrote {marketing}")

    # macOS menu bar: black on transparent; system tints the template image.
    TRAY_DIR.mkdir(parents=True, exist_ok=True)
    t16 = TRAY_DIR / "JETemplate.png"
    t32 = TRAY_DIR / "JETemplate@2x.png"
    draw_je_monogram(16, 0.78, 0.09, BLACK).save(t16, format="PNG")
    draw_je_monogram(32, 0.78, 0.09, BLACK).save(t32, format="PNG")
    print(f"Wrote {t16}")
    print(f"Wrote {t32}")

    write_macos_bundle_icons()


if __name__ == "__main__":
    main()
