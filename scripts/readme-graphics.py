#!/usr/bin/env python3
"""Generates the README graphics in assets/readme/: the title card and the
diagram of a run. The screen captures next to them come from
`cargo test --all-features -- --ignored readme_screenshots`.

Both graphics speak the language of the program: the title card is the header
card of the terminal screens, and the diagram is a `git log --graph` of what a
run does to your repository.

GitHub shows an SVG used as an image without web fonts, so every text is drawn
as outlines taken from DejaVu Sans and DejaVu Sans Mono (Bitstream Vera
licence, free to embed). Each graphic comes in a light and a dark variant and
in a wide and a narrow (phone) variant.

Usage:
    python3 scripts/readme-graphics.py [--fonts DIR] [--out DIR]

Needs fontTools (`pip install fonttools`). --fonts is the folder with
DejaVuSans.ttf, DejaVuSansMono.ttf and DejaVuSansMono-Bold.ttf; by default
matplotlib's copy is used when it is installed.
"""

import argparse
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
    """A font that measures text and turns it into one SVG path."""

    def __init__(self, path):
        self.font = TTFont(path)
        self.glyphs = self.font.getGlyphSet()
        self.cmap = self.font.getBestCmap()
        self.upm = self.font["head"].unitsPerEm
        self.hmtx = self.font["hmtx"]

    def _name(self, char):
        return self.cmap.get(ord(char), ".notdef")

    def width(self, text, size):
        return sum(self.hmtx[self._name(c)][0] for c in text) * size / self.upm

    def path(self, text, size, x, baseline):
        scale = size / self.upm
        pen = SVGPathPen(self.glyphs, ntos=lambda v: f"{v:.1f}".rstrip("0").rstrip("."))
        cursor = x
        for char in text:
            name = self._name(char)
            self.glyphs[name].draw(TransformPen(pen, (scale, 0, 0, -scale, cursor, baseline)))
            cursor += self.hmtx[name][0] * scale
        return pen.getCommands()


class Fonts:
    def __init__(self, folder):
        folder = pathlib.Path(folder)
        self.sans = Face(folder / "DejaVuSans.ttf")
        self.mono = Face(folder / "DejaVuSansMono.ttf")
        self.mono_bold = Face(folder / "DejaVuSansMono-Bold.ttf")


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


def text(face, content, size, x, baseline, color, anchor="start"):
    width = face.width(content, size)
    if anchor == "middle":
        x -= width / 2
    elif anchor == "end":
        x -= width
    return f'<path fill="{color}" d="{face.path(content, size, x, baseline)}"/>'


def stroke(d, color, width=3, dashed=False, cap="round"):
    dash = ' stroke-dasharray="2 7"' if dashed else ""
    return (
        f'<path d="{d}" fill="none" stroke="{color}" stroke-width="{width}" '
        f'stroke-linecap="{cap}"{dash}/>'
    )


def done(cx, cy, colors, radius=9):
    """A phase that passed its checks: a filled node with a check."""
    check = f"M{cx - 4} {cy + 0.5}L{cx - 1.2} {cy + 3.4}L{cx + 4.6} {cy - 3}"
    return (
        f'<circle cx="{cx}" cy="{cy}" r="{radius}" fill="{colors["accent"]}"/>'
        + stroke(check, colors["on_accent"], 2.2)
    )


def waiting(cx, cy, colors, radius=9):
    """A phase waiting for review: a ring."""
    return (
        f'<circle cx="{cx}" cy="{cy}" r="{radius - 1}" fill="none" '
        f'stroke="{colors["accent"]}" stroke-width="3"/>'
    )


def plain(cx, cy, colors, radius=6):
    """A commit that is not ours: a grey dot."""
    return f'<circle cx="{cx}" cy="{cy}" r="{radius}" fill="{colors["muted"]}"/>'


def future(cx, cy, colors, radius=8):
    return (
        f'<circle cx="{cx}" cy="{cy}" r="{radius - 1}" fill="none" '
        f'stroke="{colors["muted"]}" stroke-width="2.4" stroke-dasharray="3 3.4"/>'
    )


# --- Title card ------------------------------------------------------------

PHASES = ["Define the subcommand", "Implement it", "Cover it with tests"]


def phase_list(fonts, colors, x, middle):
    """The phase list of the run screen: one done, one in review, one pending."""
    size, step = 16, 34
    top = middle - step
    body = []
    for index, title in enumerate(PHASES):
        y = top + index * step + 5
        number = f"{index + 1}"
        if index == 0:
            body.append(text(fonts.mono_bold, "\u2713", size, x + 22, y, colors["ok"]))
            tone, face = colors["muted"], fonts.mono
        elif index == 1:
            body.append(text(fonts.mono_bold, "\u276f", size, x, y, colors["accent"]))
            body.append(text(fonts.mono_bold, "\u25c6", size, x + 22, y, colors["accent"]))
            tone, face = colors["fg"], fonts.mono_bold
        else:
            body.append(text(fonts.mono, "\u00b7", size, x + 22, y, colors["muted"]))
            tone, face = colors["muted"], fonts.mono
        body.append(text(face, number, size, x + 48, y, tone))
        body.append(text(face, title, size, x + 76, y, tone))
    return body


def card(theme, narrow, fonts):
    """The header card of the screens, large: star, name, tagline."""
    colors = THEMES[theme]
    width = 400 if narrow else 840
    pad = 28 if narrow else 44
    word = 36 if narrow else 54
    tag = 15 if narrow else 21
    meta = 12.5 if narrow else 14.5
    star_w = fonts.mono_bold.width("✻ ", word)
    base = 82 if narrow else 96
    body = [
        f'<rect x="1" y="1" width="{width - 2}" height="{{h}}" rx="14" fill="none" '
        f'stroke="{colors["line"]}" stroke-width="2"/>',
        text(fonts.mono_bold, "✻", word, pad, base, colors["accent"]),
        text(fonts.mono_bold, "phasewise", word, pad + star_w, base, colors["fg"]),
    ]
    y = base + (14 if narrow else 20) + tag
    for line in wrap(fonts.sans, TAGLINE, tag, width - pad * 2):
        body.append(text(fonts.sans, line, tag, pad, y, colors["muted"]))
        y += tag * 1.45
    y += 4
    body.append(
        text(fonts.mono, "Claude Code · Codex · git", meta, pad, y, colors["muted"])
    )
    height = int(y + pad - 4)
    if not narrow:
        body += phase_list(fonts, colors, 512, height / 2)
    body[0] = body[0].replace("{h}", str(height - 2))
    return svg(width, height, "".join(body), f"phasewise: {TAGLINE}")


# --- The run as a git graph ------------------------------------------------

LEGEND = [
    ("done", "checks passed, saved as a commit"),
    ("waiting", "waiting for your review"),
    ("future", "your merge, when you decide"),
]


def mark(kind, cx, cy, colors):
    return {"done": done, "waiting": waiting, "future": future}[kind](cx, cy, colors, 7)


def graph_wide(theme, fonts):
    colors = THEMES[theme]
    width, height = 840, 276
    main_y, run_y = 60, 166
    x_fork, x_merge = 196, 780
    xs = [340, 460, 580, 700]
    sm, md = 13, 13
    body = [
        # main: untouched
        stroke(f"M{x_fork} {main_y}H{x_merge - 8}", colors["line"], 3),
        stroke(f"M{x_merge + 8} {main_y}H{width - 14}", colors["line"], 3, dashed=True),
        # the run branch
        stroke(f"M{x_fork} {main_y}C{x_fork} {run_y - 30} {x_fork + 40} {run_y} {xs[0] - 9} {run_y}", colors["accent"]),
        stroke(f"M{xs[0]} {run_y}H{xs[3] - 9}", colors["accent"]),
        stroke(
            f"M{xs[3] + 9} {run_y}C{x_merge - 40} {run_y} {x_merge} {run_y - 30} {x_merge} {main_y + 9}",
            colors["muted"], 3, dashed=True,
        ),
        plain(x_fork, main_y, colors),
        future(x_merge, main_y, colors),
    ]
    for index, x in enumerate(xs):
        body.append(done(x, run_y, colors) if index < 3 else waiting(x, run_y, colors))
        body.append(text(fonts.mono_bold, f"phase {index + 1}", md, x, run_y + 34, colors["fg"], "middle"))
        label = HASHES[index] if index < 3 else "in review"
        tone = colors["muted"] if index < 3 else colors["accent"]
        body.append(text(fonts.mono, label, 12, x, run_y + 54, tone, "middle"))
    # Lane names.
    body += [
        text(fonts.mono_bold, "main", sm, 0, main_y + 5, colors["fg"]),
        text(fonts.sans, "your branch", 12.5, 0, main_y + 26, colors["muted"]),
        text(fonts.mono_bold, "phasewise/", sm, 0, run_y - 2, colors["fg"]),
        text(fonts.mono_bold, "<run-id>", sm, 0, run_y + 17, colors["fg"]),
        text(fonts.sans, "untouched until you merge", 12.5, (x_fork + x_merge) / 2, main_y - 18, colors["muted"], "middle"),
    ]
    # Legend.
    x = 0
    for kind, label in LEGEND:
        body.append(mark(kind, x + 8, 250, colors))
        body.append(text(fonts.sans, label, 12.5, x + 24, 254.5, colors["muted"]))
        x += 24 + fonts.sans.width(label, 12.5) + 30
    return svg(width, height, "".join(body), GRAPH_ALT)


def graph_narrow(theme, fonts):
    colors = THEMES[theme]
    width, height = 400, 452
    main_x, run_x = 14, 128
    top_y, merge_y = 40, 396
    ys = [152, 214, 276, 338]
    body = [
        stroke(f"M{main_x} {top_y}V{merge_y - 9}", colors["line"], 3),
        stroke(f"M{main_x} {top_y}C{main_x} {ys[0] - 40} {run_x} {top_y + 30} {run_x} {ys[0] - 9}", colors["accent"]),
        stroke(f"M{run_x} {ys[0]}V{ys[3] - 9}", colors["accent"]),
        stroke(
            f"M{run_x} {ys[3] + 9}C{run_x} {merge_y - 30} {main_x} {ys[3] + 30} {main_x} {merge_y - 9}",
            colors["muted"], 3, dashed=True,
        ),
        plain(main_x, top_y, colors),
        future(main_x, merge_y, colors),
        text(fonts.mono_bold, "main", 13, main_x + 26, top_y + 5, colors["fg"]),
        text(fonts.sans, "your branch, untouched until you merge", 12.5, main_x + 26, top_y + 25, colors["muted"]),
        text(fonts.mono_bold, "phasewise/<run-id>", 13, run_x + 26, ys[0] - 36, colors["fg"]),
        text(fonts.mono_bold, "merge", 13, main_x + 26, merge_y + 5, colors["fg"]),
        text(fonts.sans, "when you decide", 12.5, main_x + 26, merge_y + 25, colors["muted"]),
    ]
    for index, y in enumerate(ys):
        body.append(done(run_x, y, colors) if index < 3 else waiting(run_x, y, colors))
        body.append(text(fonts.mono_bold, f"phase {index + 1}", 13, run_x + 26, y + 5, colors["fg"]))
        label = HASHES[index] if index < 3 else "in review"
        tone = colors["muted"] if index < 3 else colors["accent"]
        body.append(text(fonts.mono, label, 12, run_x + 26, y + 24, tone))
    y = 430
    return svg(width, height, "".join(body), GRAPH_ALT)


GRAPH_ALT = (
    "Your branch, main, stays untouched. A run forks the branch phasewise/<run-id> "
    "from it. Phases 1 to 3 passed their checks and are saved as commits; phase 4 is "
    "in review. When you decide, you merge the run branch into main."
)


def default_fonts():
    try:
        import matplotlib

        return pathlib.Path(matplotlib.get_data_path()) / "fonts" / "ttf"
    except ImportError:
        return None


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--fonts", type=pathlib.Path, default=default_fonts())
    parser.add_argument(
        "--out", type=pathlib.Path, default=pathlib.Path(__file__).parent.parent / "assets" / "readme"
    )
    args = parser.parse_args()
    if args.fonts is None or not (args.fonts / "DejaVuSans.ttf").exists():
        sys.exit("DejaVu fonts not found: pass --fonts DIR (see the module docstring).")
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
