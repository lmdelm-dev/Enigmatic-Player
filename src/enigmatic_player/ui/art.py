"""Render embedded album art as pixel-art.

Album covers are reduced to a 4-shade mint-green palette (dark -> bright,
matching the matrix-on-black theme) with ordered (Bayer 4x4) dithering, then
drawn as half-block cells for a chunky pixel look.
"""

from __future__ import annotations

import io
import random
from typing import List, Optional, Tuple

# lightest -> darkest spring greens, tuned for a black background
GREENSHADES = [(160, 255, 210), (70, 220, 140), (16, 120, 78), (4, 32, 20)]

# gruvbox palette for random shapes
GB_BG = "#282828"
GB_SHADES = ["#1d2021", "#3c3836", "#504945", "#665c54", "#7c6f64",
             "#a89984", "#bdae93", "#d5c4a1", "#ebdbb2", "#fbf1c7",
             "#fb4934", "#fe8019", "#fabd2f", "#b8bb26", "#8ec07c",
             "#83a598", "#d3869b"]

BAYER4 = [
    [0, 8, 2, 10],
    [12, 4, 14, 6],
    [3, 11, 1, 9],
    [15, 7, 13, 5],
]


def _rgb(marker: str, pix: Tuple[int, int, int]) -> str:
    r, g, b = pix
    return f"[{marker}rgb({r},{g},{b})]"


def _shade_map(img, cols: int, rows2: int) -> List[List[int]]:
    """Return a cols x rows2 grid of shade indices 0..3 (lightest -> darkest)."""
    px = img.load()
    out: List[List[int]] = []
    for y in range(rows2):
        row = []
        for x in range(cols):
            lum = px[x, y]
            level = min(1.0, max(0.0, lum / 255.0))
            dither = (BAYER4[y % 4][x % 4] + 0.5) / 16.0 - 0.5
            q = int(min(3, max(0, level * 4.0 + dither)))
            row.append(q)
        out.append(row)
    return out


def render_gameboy(art_bytes: bytes, cols: int = 16) -> Optional[str]:
    """Render cover art as dither-mapped Game Boy cells (Rich markup lines)."""
    try:
        from PIL import Image
    except ImportError:
        return None
    try:
        img = Image.open(io.BytesIO(art_bytes)).convert("L")
    except Exception:  # noqa: BLE001
        return None

    aspect = img.height / img.width
    rows2 = max(4, round(cols * aspect * 2.0))
    img = img.resize((cols, rows2), Image.Resampling.LANCZOS)
    grid = _shade_map(img, cols, rows2)

    lines = []
    for r in range(0, rows2, 2):
        cells = []
        for x in range(cols):
            top = grid[r][x]
            bot = grid[r + 1][x] if r + 1 < rows2 else top
            ct = GREENSHADES[top]
            cb = GREENSHADES[bot]
            if top == bot:
                cells.append(f"[on rgb({ct[0]},{ct[1]},{ct[2]})] ")
            else:
                cells.append(
                    f"[rgb({ct[0]},{ct[1]},{ct[2]}) on rgb({cb[0]},{cb[1]},{cb[2]})]▀"
                )
        lines.append("".join(cells))
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------------------
#  Random placeholder shapes — a new one for every track
# ---------------------------------------------------------------------------
COLS, ROWS2 = 16, 12


def _cell(top: str, bot: str) -> str:
    if top == bot:
        return f"[on {top}] [/]"
    return f"[{top} on {bot}]▀"


def _rand_fg() -> str:
    return random.choice(GB_SHADES)


def _shape_diamond() -> List[List[str]]:
    grid = [[GB_BG] * COLS for _ in range(ROWS2)]
    cx, cy = COLS // 2, ROWS2 // 2
    r = random.randint(3, 5)
    c1, c2 = _rand_fg(), _rand_fg()
    for y in range(ROWS2):
        for x in range(COLS):
            if abs(x - cx) + abs(y - cy) <= r:
                grid[y][x] = random.choice([c1, c2])
    return grid


def _shape_cross() -> List[List[str]]:
    grid = [[GB_BG] * COLS for _ in range(ROWS2)]
    cx, cy = COLS // 2, ROWS2 // 2
    w = random.randint(1, 2)
    c1, c2 = _rand_fg(), _rand_fg()
    for y in range(ROWS2):
        for x in range(COLS):
            if abs(x - cx) <= w or abs(y - cy) <= w:
                grid[y][x] = random.choice([c1, c2])
    return grid


def _shape_circle() -> List[List[str]]:
    grid = [[GB_BG] * COLS for _ in range(ROWS2)]
    cx, cy = COLS // 2, ROWS2 // 2
    r = random.randint(3, 5)
    c1, c2 = _rand_fg(), _rand_fg()
    for y in range(ROWS2):
        for x in range(COLS):
            d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5
            if d <= r:
                grid[y][x] = random.choice([c1, c2])
            elif d <= r + 1.5:
                grid[y][x] = GB_SHADES[3]
    return grid


def _shape_mountain() -> List[List[str]]:
    grid = [[GB_BG] * COLS for _ in range(ROWS2)]
    peak = random.randint(COLS // 3, COLS * 2 // 3)
    h = random.randint(4, 7)
    c1, c2 = _rand_fg(), _rand_fg()
    for x in range(COLS):
        height = max(0, h - abs(x - peak) // 2)
        for y in range(ROWS2 - height, ROWS2):
            if y >= 0:
                grid[y][x] = random.choice([c1, c2])
    return grid


def _shape_wave() -> List[List[str]]:
    grid = [[GB_BG] * COLS for _ in range(ROWS2)]
    c1, c2 = _rand_fg(), _rand_fg()
    amp = random.randint(1, 3)
    freq = random.random() * 0.8 + 0.3
    phase = random.random() * 6.28
    for x in range(COLS):
        mid = ROWS2 // 2 + int(amp * (0.5 + 0.5 * (1 if (x * freq + phase) % 6.28 < 3.14 else -1)))
        for y in range(mid, ROWS2):
            if 0 <= y < ROWS2:
                grid[y][x] = random.choice([c1, c2])
    return grid


def _shape_checker() -> List[List[str]]:
    grid = [[GB_BG] * COLS for _ in range(ROWS2)]
    c1, c2 = _rand_fg(), _rand_fg()
    block = random.choice([2, 3])
    for y in range(ROWS2):
        for x in range(COLS):
            if (x // block + y // block) % 2 == 0:
                grid[y][x] = c1
            else:
                grid[y][x] = c2
    return grid


def _shape_stripes() -> List[List[str]]:
    grid = [[GB_BG] * COLS for _ in range(ROWS2)]
    c1, c2 = _rand_fg(), _rand_fg()
    w = random.randint(1, 3)
    for y in range(ROWS2):
        for x in range(COLS):
            if (x // w) % 2 == 0:
                grid[y][x] = c1
            else:
                grid[y][x] = c2
    return grid


def _shape_noise() -> List[List[str]]:
    grid = [[GB_BG] * COLS for _ in range(ROWS2)]
    for y in range(ROWS2):
        for x in range(COLS):
            if random.random() < 0.35:
                grid[y][x] = _rand_fg()
    return grid


def _shape_dots() -> List[List[str]]:
    grid = [[GB_BG] * COLS for _ in range(ROWS2)]
    c1 = _rand_fg()
    spacing = random.choice([3, 4, 5])
    for y in range(ROWS2):
        for x in range(COLS):
            if x % spacing == 0 and y % spacing == 0:
                grid[y][x] = c1
    return grid


def _shape_star() -> List[List[str]]:
    grid = [[GB_BG] * COLS for _ in range(ROWS2)]
    cx, cy = COLS // 2, ROWS2 // 2
    c1, c2 = _rand_fg(), _rand_fg()
    for y in range(ROWS2):
        for x in range(COLS):
            dx, dy = abs(x - cx), abs(y - cy)
            if dx == 0 or dy == 0 or dx == dy:
                grid[y][x] = random.choice([c1, c2])
    return grid


SHAPES = [
    _shape_diamond, _shape_cross, _shape_circle, _shape_mountain,
    _shape_wave, _shape_checker, _shape_stripes, _shape_noise,
    _shape_dots, _shape_star,
]


def random_placeholder() -> str:
    """Return a random pixel-art shape rendered as Rich markup."""
    grid = random.choice(SHAPES)()
    lines = []
    for r in range(0, ROWS2, 2):
        cells = []
        for x in range(COLS):
            cells.append(_cell(grid[r][x], grid[r + 1][x]))
        lines.append("".join(cells))
    return "\n".join(lines) + "\n"


PLACEHOLDER = random_placeholder()
