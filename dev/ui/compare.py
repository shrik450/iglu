"""Shows what changed between two tours.

    python3 dev/ui/compare.py BEFORE AFTER

BEFORE and AFTER are tour labels under .dev/tour; stops only one of them
visited are skipped. It writes
.dev/tour/compare-BEFORE-AFTER/index.html with each changed screenshot before,
after, and with its changed pixels marked, most changed first, and prints the
same list. A fix should change what it meant to and nothing else.
"""

import html
import sys
from pathlib import Path

from PIL import Image, ImageChops

ROOT = Path(__file__).resolve().parents[2]
TOURS = ROOT / ".dev/tour"


def changed(before: Path, after: Path, mark: Path) -> float:
    """The share of pixels that differ, saving `after` with them marked."""
    a = Image.open(before).convert("RGB")
    b = Image.open(after).convert("RGB")
    if a.size != b.size:
        canvas = Image.new("RGB", b.size)
        canvas.paste(a)
        a = canvas
    mask = ImageChops.difference(a, b).convert("L").point(lambda v: 255 if v > 16 else 0)
    share = mask.histogram()[255] / (b.size[0] * b.size[1])
    if share:
        dimmed = Image.blend(b, Image.new("RGB", b.size), 0.6)
        dimmed.paste(Image.new("RGB", b.size, (255, 40, 40)), mask=mask)
        dimmed.save(mark)
    return share


def main() -> None:
    before_label, after_label = sys.argv[1], sys.argv[2]
    before, after = TOURS / before_label, TOURS / after_label
    out = TOURS / f"compare-{before_label}-{after_label}"
    out.mkdir(parents=True, exist_ok=True)
    names = {p.name for p in before.glob("*.png")} | {p.name for p in after.glob("*.png")}
    rows, same, alone = [], 0, 0
    for name in sorted(names):
        # A stop only one tour visited, such as after a tour with --only.
        if not (before / name).exists() or not (after / name).exists():
            alone += 1
            continue
        share = changed(before / name, after / name, out / name)
        if share:
            rows.append((share, name, f"{share:.1%} changed"))
        else:
            same += 1
    rows.sort(reverse=True)
    for _, name, what in rows:
        print(f"{what:>14}  {name}")
    print(f"{same} unchanged, {alone} in only one tour")

    def img(tour: Path, name: str) -> str:
        path = tour / name
        return f"<a href='{path}'><img loading=lazy src='{path}'></a>" if path.exists() else "<i>none</i>"

    body = "".join(
        f"<section><h2>{html.escape(name)} <small>{what}</small></h2><div class=row>"
        f"{img(before, name)}{img(after, name)}{img(out, name)}</div></section>"
        for _, name, what in rows
    )
    (out / "index.html").write_text(f"""<!doctype html><meta charset=utf-8><title>{before_label} → {after_label}</title>
<style>body{{font:14px system-ui;margin:16px;background:#111;color:#ddd}}.row{{display:flex;gap:12px;overflow-x:auto}}
img{{height:360px;border:1px solid #333}}small{{color:#999;font-weight:normal}}</style>
<h1>{html.escape(before_label)} → {html.escape(after_label)}</h1><p>before, after, and changed pixels in red. {same} unchanged, {alone} in only one tour.</p>{body}""")
    print(out / "index.html")


if __name__ == "__main__":
    main()
