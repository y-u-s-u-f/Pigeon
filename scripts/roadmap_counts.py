#!/usr/bin/env python3
"""Recompute the Progress table in docs/ROADMAP.md from the status cells of
each area's table. Run after changing any status."""

import pathlib
import re

path = pathlib.Path(__file__).resolve().parent.parent / "docs" / "ROADMAP.md"
text = path.read_text()

STATUSES = ["⚪", "🟡", "🟢"]
counts = {}
for m in re.finditer(r"^\| ([A-Z][A-Z0-9]{1,3})-[0-9]+[a-z]? \|.*\| (⚪|🟡|🟢) [^|]*\|", text, re.M):
    counts.setdefault(m.group(1), {s: 0 for s in STATUSES})[m.group(2)] += 1

total = {s: 0 for s in STATUSES}


def row(m):
    name, code = m.group(1), m.group(2).upper()
    c = counts.get(code, {s: 0 for s in STATUSES})
    for s in STATUSES:
        total[s] += c[s]
    return f"| {name} | {c['⚪']} | {c['🟡']} | {c['🟢']} | {sum(c.values())} |"


text = re.sub(r"^\| (\[[^\]]+\]\(#[^)]*-([a-z][a-z0-9]{1,3})\)) \| \d+ \| \d+ \| \d+ \| \d+ \|$", row, text, flags=re.M)
text = re.sub(
    r"^\| \*\*Overall\*\* \|.*$",
    f"| **Overall** | **{total['⚪']}** | **{total['🟡']}** | **{total['🟢']}** | **{sum(total.values())}** |",
    text,
    flags=re.M,
)
path.write_text(text)
print(f"⚪ {total['⚪']}  🟡 {total['🟡']}  🟢 {total['🟢']}  total {sum(total.values())}")
