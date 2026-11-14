#!/usr/bin/env python3
"""Generates the README graphics in assets/readme/: the title card and the
diagram of a run. The screen captures next to them come from
`cargo test --all-features -- --ignored readme_screenshots`.

Both graphics speak the language of the program: the title card is the header
card of the terminal screens, and the diagram is a `git log --graph` of what a
run does to your repository.

GitHub shows an SVG used as an image without web fonts, so every text is drawn
as outlines. Prose is Source Sans Pro Regular and the program's own words
(wordmark, branch names, hashes) are Inconsolata Regular, both under the SIL
Open Font License, which allows embedding. The few symbols neither has (the
star, the prompt arrow) come from DejaVu Sans Mono. Nothing is bold: emphasis
comes from size and colour. Each graphic comes in a light and a dark variant
and in a wide and a narrow (phone) variant.

Usage:
    python3 scripts/readme-graphics.py --fonts DIR [--out DIR]

Needs fontTools (`pip install fonttools`). DIR holds SourceSansPro-Regular.ttf,
Inconsolata-Regular.ttf and DejaVuSansMono.ttf.
"""

import argparse
import math
import pathlib
import sys
from xml.sax.saxutils import escape

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

# The palette of the terminal screens: cyan for what matters, grey for the rest.
THEMES = {
    "light": {
        "fg": "#1f2328",
        "muted": "#59636e",
        "accent": "#0e7490",
        "line": "#c4ccd4",
        "ok": "#1a7f37",
        "on_accent": "#ffffff",
    },
    "dark": {
        "fg": "#f0f6fc",
        "muted": "#9198a1",
        "accent": "#22d3ee",
        "line": "#3d444d",
        "ok": "#3fb950",
        "on_accent": "#0d1117",
    },
}

TAGLINE = "Big plans, one verified phase at a time."
HASHES = ["a3f9c21", "5be2d07", "91c4e3a"]


class Face:
    """A font that measures text and turns it into one SVG path. Characters the
    font lacks are drawn from the fallback, scaled to the same size."""

    def __init__(self, path, fallback=None):
        self.font = TTFont(path)
        self.glyphs = self.font.getGlyphSet()
        self.cmap = self.font.getBestCmap()
        self.upm = self.font["head"].unitsPerEm
        self.hmtx = self.font["hmtx"]
        os2 = self.font["OS/2"]
        self.cap = getattr(os2, "sCapHeight", 0) / self.upm or 0.7
        self.xheight = getattr(os2, "sxHeight", 0) / self.upm or 0.5
        self.fallback = fallback

    def _source(self, char):
        if ord(char) not in self.cmap and self.fallback and ord(char) in self.fallback.cmap:
            return self.fallback._source(char)
        return self, self.cmap.get(ord(char), ".notdef")

    def width(self, text, size):
        total = 0.0
        for char in text:
            face, name = self._source(char)
            total += face.hmtx[name][0] * size / face.upm
        return total

    def path(self, text, size, x, baseline):
        pen = SVGPathPen(None, ntos=lambda v: f"{v:.2f}".rstrip("0").rstrip("."))
        cursor = x
        for char in text:
            face, name = self._source(char)
            scale = size / face.upm
            pen.glyphSet = face.glyphs
            face.glyphs[name].draw(TransformPen(pen, (scale, 0, 0, -scale, cursor, baseline)))
            cursor += face.hmtx[name][0] * scale
        return pen.getCommands()

    def centre_offset(self, size):
        """Distance from the middle of a line of text to its baseline: the
        middle of the lowercase body and the capitals, which is where the eye
        puts it."""
        return (self.cap + self.xheight) / 4 * size


class Fonts:
    def __init__(self, folder):
        folder = pathlib.Path(folder)
        symbols = Face(folder / "DejaVuSansMono.ttf")
        self.sans = Face(folder / "SourceSansPro-Regular.ttf", symbols)
        self.mono = Face(folder / "Inconsolata-Regular.ttf", symbols)


def wrap(face, text, size, max_width):
    lines, current = [], ""
    for word in text.split():
        trial = word if not current else f"{current} {word}"
        if current and face.width(trial, size) > max_width:
            lines.append(current)
            current = word
        else:
            current = trial
    if current:
        lines.append(current)
    return lines


def svg(width, height, body, description):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}" role="img"><title>{escape(description)}</title>{body}</svg>\n'
    )


def place(face, content, size, x, anchor):
    width = face.width(content, size)
    if anchor == "middle":
        return x - width / 2
    if anchor == "end":
        return x - width
    return x


def text(face, content, size, x, baseline, color, anchor="start", opacity=None):
    """Text whose baseline sits at the given y."""
    extra = f' fill-opacity="{opacity}"' if opacity is not None else ""
    return f'<path fill="{color}"{extra} d="{face.path(content, size, place(face, content, size, x, anchor), baseline)}"/>'


def label(face, content, size, x, middle, color, anchor="start", opacity=None):
    """Text whose visual middle sits at the given y."""
    return text(face, content, size, x, middle + face.centre_offset(size), color, anchor, opacity)


def stroke(d, color, width=3, dashed=False, cap="round"):
    dash = ' stroke-dasharray="1 7"' if dashed else ""
    return (
        f'<path d="{d}" fill="none" stroke="{color}" stroke-width="{width}" '
        f'stroke-linecap="{cap}" stroke-linejoin="round"{dash}/>'
    )


def arc(radius, sweep, dx, dy):
    """A circular arc to the point (dx, dy) away from the current one. Every
    bend in the graphics is a true quarter circle that meets the straight
    lines on both sides tangentially."""
    return f"a{radius} {radius} 0 0 {sweep} {dx} {dy}"


def done(cx, cy, colors, radius=9):
    """A phase that passed its checks: a filled node with a check."""
    k = radius / 9
    check = (
        f"M{cx - 4 * k:.2f} {cy + 0.2 * k:.2f}"
        f"L{cx - 1.2 * k:.2f} {cy + 3 * k:.2f}"
        f"L{cx + 4.2 * k:.2f} {cy - 3 * k:.2f}"
    )
    return (
        f'<circle cx="{cx}" cy="{cy}" r="{radius}" fill="{colors["accent"]}"/>'
        + stroke(check, colors["on_accent"], 2.2 * k)
    )


def waiting(cx, cy, colors, radius=9):
    """A phase waiting for review: a ring."""
    return (
        f'<circle cx="{cx}" cy="{cy}" r="{radius - 1.5}" fill="none" '
        f'stroke="{colors["accent"]}" stroke-width="3"/>'
    )


def plain(cx, cy, colors, radius=6):
    """A commit that is not ours: a grey dot."""
    return f'<circle cx="{cx}" cy="{cy}" r="{radius}" fill="{colors["muted"]}"/>'


def future(cx, cy, colors, radius=9):
    """The merge you have not made yet: a dotted ring."""
    r = radius - 1.25
    pitch = 2 * math.pi * r / 8  # eight dots, evenly spaced, no seam
    return (
        f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" '
        f'stroke="{colors["muted"]}" stroke-width="2.5" '
        f'stroke-dasharray="0.01 {pitch - 0.01:.3f}" stroke-linecap="round" '
        f'transform="rotate(-90 {cx} {cy})"/>'
    )


# --- Title card ------------------------------------------------------------

PHASES = ["Define the subcommand", "Implement it", "Cover it with tests"]


def phase_list(fonts, colors, x, middle):
    """The phase list of the run screen on a character grid: prompt, status,
    number, title. One done, one in review, one pending."""
    size, step = 17, 36
    cell = fonts.mono.width("0", size)
    top = middle - step
    body = []

    def cell_at(column):
        return x + (column + 0.5) * cell

    for index, title in enumerate(PHASES):
        y = top + index * step
        number = f"{index + 1}"
        if index == 0:
            body.append(label(fonts.mono, "✓", size, cell_at(2), y, colors["ok"], "middle"))
            tone, opacity = colors["muted"], None
        elif index == 1:
            body.append(label(fonts.mono, "❯", size, cell_at(0), y, colors["accent"], "middle"))
            body.append(label(fonts.mono, "◆", size, cell_at(2), y, colors["accent"], "middle"))
            tone, opacity = colors["fg"], None
        else:
            body.append(label(fonts.mono, "·", size, cell_at(2), y, colors["muted"], "middle", 0.7))
            tone, opacity = colors["muted"], 0.7
        body.append(label(fonts.mono, number, size, cell_at(4), y, tone, "middle", opacity))
        body.append(label(fonts.mono, title, size, x + 6 * cell, y, tone, "start", opacity))
    return body


def card(theme, narrow, fonts):
    """The header card of the screens, large: star, name, tagline."""
    colors = THEMES[theme]
    width = 400 if narrow else 840
    pad = 30 if narrow else 48
    pad_y = 34 if narrow else 44
    word = 40 if narrow else 58
    tag = 16 if narrow else 22
    meta = 13 if narrow else 15
    mono = fonts.mono

    # Stack the text block from the top of the capitals of the wordmark down
    # to the baseline of the last line, so the padding above and below is equal.
    base = pad_y + mono.cap * word
    star_w = mono.width("✻ ", word)
    body = [
        text(mono, "✻", word, pad, base, colors["accent"]),
        text(mono, "phasewise", word, pad + star_w, base, colors["fg"]),
    ]
    y = base + (18 if narrow else 24) + fonts.sans.cap * tag
    lines = wrap(fonts.sans, TAGLINE, tag, width - pad * 2)
    for line in lines:
        body.append(text(fonts.sans, line, tag, pad, y, colors["muted"]))
        y += tag * 1.4
    y += (4 if narrow else 8)
    body.append(text(mono, "Claude Code · Codex · git", meta, pad, y, colors["muted"], opacity=0.85))
    height = round(y + pad_y - (6 if narrow else 4))
    frame = (
        f'<rect x="1" y="1" width="{width - 2}" height="{height - 2}" rx="14" fill="none" '
        f'stroke="{colors["line"]}" stroke-width="2"/>'
    )
    if not narrow:
        divider_x = 496
        body.append(stroke(f"M{divider_x} {pad_y}V{height - pad_y}", colors["line"], 1.5, cap="butt"))
        body += phase_list(fonts, colors, 536, height / 2)
    return svg(width, height, frame + "".join(body), f"phasewise: {TAGLINE}")


# --- The run as a git graph ------------------------------------------------

LEGEND = [
    ("done", "checks passed, saved as a commit"),
    ("waiting", "waiting for your review"),
    ("future", "your merge, when you decide"),
]

NAME, NOTE, CAPTION = 14, 13.5, 12.5
BEND = 36  # radius of every bend in the run graphs


def mark(kind, cx, cy, colors):
    return {"done": done, "waiting": waiting, "future": future}[kind](cx, cy, colors, 7.5)


def legend(fonts, colors, x, middle):
    body = []
    for kind, words in LEGEND:
        body.append(mark(kind, x + 8, middle, colors))
        body.append(label(fonts.sans, words, NOTE, x + 24, middle, colors["muted"]))
        x += 24 + fonts.sans.width(words, NOTE) + 32
    return body


def graph_wide(theme, fonts):
    colors = THEMES[theme]
    width, height = 840, 280
    main_y, run_y = 62, 168
    x_fork, x_merge = 216, 780
    xs = [348, 468, 588, 708]
    body = [
        # main: untouched until the merge, then it carries on.
        stroke(f"M{x_fork} {main_y}H{x_merge - 12}", colors["line"], 3),
        stroke(f"M{x_merge + 12} {main_y}H{width - 3}", colors["line"], 3, dashed=True),
        # The run branch leaves main at the fork and runs through the phases.
        stroke(f"M{x_fork} {main_y}V{run_y - BEND}{arc(BEND, 0, BEND, BEND)}H{xs[3] - 12}", colors["accent"]),
        # The merge: not made yet.
        stroke(
            f"M{xs[3] + 12} {run_y}H{x_merge - BEND}{arc(BEND, 0, BEND, -BEND)}V{main_y + 12}",
            colors["muted"], 3, dashed=True,
        ),
        plain(x_fork, main_y, colors),
        future(x_merge, main_y, colors, 9),
    ]
    for index, x in enumerate(xs):
        body.append(done(x, run_y, colors) if index < 3 else waiting(x, run_y, colors))
        body.append(label(fonts.mono, f"phase {index + 1}", NAME, x, run_y + 36, colors["fg"], "middle"))
        words = HASHES[index] if index < 3 else "in review"
        tone = colors["muted"] if index < 3 else colors["accent"]
        body.append(label(fonts.mono, words, CAPTION + 0.5, x, run_y + 56, tone, "middle"))
    # Lane names, each a name over a plain note, centred on its lane.
    body += [
        label(fonts.mono, "main", NAME, 0, main_y - 10, colors["fg"]),
        label(fonts.sans, "your branch", NOTE, 0, main_y + 11, colors["muted"]),
        label(fonts.mono, "phasewise/<run-id>", NAME, 0, run_y - 10, colors["fg"]),
        label(fonts.sans, "the run\u2019s own branch", NOTE, 0, run_y + 11, colors["muted"]),
        label(fonts.sans, "untouched until you merge", NOTE, (x_fork + x_merge) / 2, main_y - 22, colors["muted"], "middle"),
    ]
    body += legend(fonts, colors, 0, height - 30)
    return svg(width, height, "".join(body), GRAPH_ALT)


def graph_narrow(theme, fonts):
    colors = THEMES[theme]
    width, height = 400, 452
    main_x, run_x = 12, 120
    top_y, merge_y = 36, 388
    ys = [148, 208, 268, 328]
    text_x = lambda x: x + 26
    bend = 28
    body = [
        stroke(f"M{main_x} {top_y}V{merge_y - 12}", colors["line"], 3),
        # Down from main, across, and down again onto the run's own lane.
        stroke(
            f"M{main_x} {top_y}V{ys[0] - 12 - 2 * bend}{arc(bend, 0, bend, bend)}"
            f"H{run_x - bend}{arc(bend, 1, bend, bend)}V{ys[0] - 12}",
            colors["accent"],
        ),
        stroke(f"M{run_x} {ys[0]}V{ys[3] - 12}", colors["accent"]),
        # The merge, not made yet: down, round the corner, and into main.
        stroke(
            f"M{run_x} {ys[3] + 12}V{merge_y - bend}{arc(bend, 1, -bend, bend)}H{main_x + 13}",
            colors["muted"], 3, dashed=True,
        ),
        plain(main_x, top_y, colors),
        future(main_x, merge_y, colors, 9),
        label(fonts.mono, "main", NAME, text_x(main_x), top_y - 10, colors["fg"]),
        label(fonts.sans, "your branch, untouched until you merge", NOTE, text_x(main_x), top_y + 11, colors["muted"]),
        label(fonts.mono, "phasewise/<run-id>", NAME, text_x(run_x), ys[0] - 46, colors["fg"]),
        label(fonts.mono, "merge", NAME, text_x(main_x), merge_y + 26, colors["fg"]),
        label(fonts.sans, "your merge, when you decide", NOTE, text_x(main_x), merge_y + 47, colors["muted"]),
    ]
    for index, y in enumerate(ys):
        body.append(done(run_x, y, colors) if index < 3 else waiting(run_x, y, colors))
        body.append(label(fonts.mono, f"phase {index + 1}", NAME, text_x(run_x), y - 10, colors["fg"]))
        words = HASHES[index] if index < 3 else "in review"
        tone = colors["muted"] if index < 3 else colors["accent"]
        body.append(label(fonts.mono, words, CAPTION + 0.5, text_x(run_x), y + 11, tone))
    return svg(width, height, "".join(body), GRAPH_ALT)


GRAPH_ALT = (
    "Your branch, main, stays untouched. A run forks the branch phasewise/<run-id> "
    "from it. Phases 1 to 3 passed their checks and are saved as commits; phase 4 is "
    "in review. When you decide, you merge the run branch into main."
)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--fonts", type=pathlib.Path, required=True)
    parser.add_argument(
        "--out", type=pathlib.Path, default=pathlib.Path(__file__).parent.parent / "assets" / "readme"
    )
    args = parser.parse_args()
    needed = ["SourceSansPro-Regular.ttf", "Inconsolata-Regular.ttf", "DejaVuSansMono.ttf"]
    missing = [name for name in needed if not (args.fonts / name).exists()]
    if missing:
        sys.exit(f"missing in {args.fonts}: {', '.join(missing)} (see the module docstring)")
    fonts = Fonts(args.fonts)
    args.out.mkdir(parents=True, exist_ok=True)
    count = 0
    for theme in THEMES:
        for narrow in (False, True):
            variant = "narrow" if narrow else "wide"
            graph = graph_narrow if narrow else graph_wide
            files = {
                f"card-{variant}-{theme}.svg": card(theme, narrow, fonts),
                f"run-graph-{variant}-{theme}.svg": graph(theme, fonts),
            }
            for name, content in files.items():
                (args.out / name).write_text(content, encoding="utf-8")
                count += 1
    print(f"wrote {count} files to {args.out}")


if __name__ == "__main__":
    main()
