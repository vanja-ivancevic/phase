#!/usr/bin/env python3
"""Measure how many decklists are fully playable from a coverage report.

A deck is "fully playable" when every card in its decklist is `supported` by
the engine's parse coverage (the same per-card support the coverage report
already computes). This is the deck-level companion to `coverage-report`:
it joins a coverage report (or its `card-data.json`) with a directory of
Forge `.dck` decklists and prints the fully-supported deck count plus the
remaining blockers ranked by the number of decklists they appear in.

Usage:
    deck-coverage.py <coverage-dir-or-report> <decks-dir> [--json out.json]

`<coverage-dir-or-report>` may be:
  * a run directory containing `card-data.json` plus a saved report log
    (`--brief` output), or
  * a path to a report log itself.

Card names are matched with unicode folding (NFKD, combining marks stripped)
so `Lim-Dûl's Vault` matches `Lim-Dul's Vault`, and split cards are matched
per face (`Fire // Ice` requires `Fire` and `Ice`). Removed-offensive cards
(e.g. Crusade) are absent from the export by design and report as blockers.

Exit code is 0 always; the summary is the output. This is a diagnostic tool,
not a gate.
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import re
import sys
import unicodedata


def fold(name: str) -> str:
    """Case/unicode-fold a card name for cross-source matching."""
    n = unicodedata.normalize("NFKD", name)
    n = "".join(ch for ch in n if not unicodedata.combining(ch))
    return re.sub(r"\s+", " ", n).strip().lower()


def load_supported(corpus: str) -> dict[str, bool]:
    """supported-flag map keyed by folded card name."""
    paths = []
    if os.path.isdir(corpus):
        paths = sorted(glob.glob(os.path.join(corpus, "*.log")))
        paths = [p for p in paths if os.path.getsize(p) > 1_000_000]
    else:
        paths = [corpus]
    if not paths:
        sys.exit(f"no coverage report found under {corpus}")
    # Prefer the largest log — the brief report JSON is the bulk of it.
    path = max(paths, key=os.path.getsize)
    text = open(path, errors="replace").read()
    start = text.index("\n{") + 1
    end = text.rindex("\n}")
    doc = json.loads(text[start : end + 2])
    supported: dict[str, bool] = {}
    for card in doc["cards"]:
        supported[fold(card["card_name"])] = bool(card["supported"])
    return supported


def deck_card_faces(path: str) -> list[list[str]]:
    """Card-name face lists (one entry per deck line) from a Forge .dck."""
    out: list[list[str]] = []
    for raw in open(path, errors="replace"):
        line = raw.strip()
        if not line or line.startswith(("//", "#")):
            continue
        m = re.match(r"^(?:[A-Za-z0-9:]+:)?\s*\d+\s+(.+)$", line)
        if not m:
            continue
        name = m.group(1).strip()
        head = name.split("[")[0]
        if "//" in head:
            parts = head.split("//")
        elif "/" in head:
            parts = head.split("/")
        else:
            parts = [name]
        faces = [fold(p) for p in (x.strip().rstrip(".").strip('"') for x in parts)]
        if faces and all(faces):
            out.append(faces)
    return out


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("coverage", help="coverage run dir or report log")
    ap.add_argument("decks", help="directory of .dck decklists")
    ap.add_argument("--json", dest="json_out", help="write the summary as JSON")
    args = ap.parse_args()

    supported = load_supported(args.coverage)
    deck_files = sorted(glob.glob(os.path.join(args.decks, "*.dck")))
    if not deck_files:
        sys.exit(f"no .dck files under {args.decks}")

    full: list[str] = []
    partial: list[tuple[str, list[str]]] = []
    impact: dict[str, int] = {}
    for path in deck_files:
        deck = os.path.basename(path)[:-4]
        missing: set[str] = set()
        for faces in deck_card_faces(path):
            if not all(supported.get(f, False) for f in faces):
                missing.add("/".join(faces))
        if missing:
            partial.append((deck, sorted(missing)))
            for name in missing:
                impact[name] = impact.get(name, 0) + 1
        else:
            full.append(deck)

    total = len(full) + len(partial)
    print(f"FULLY SUPPORTED DECKS: {len(full)}/{total}")
    if partial:
        print()
        print("=== blockers by number of decklists ===")
        for name, count in sorted(impact.items(), key=lambda kv: (-kv[1], kv[0])):
            print(f"  {count:3d} decks | {name}")
        print()
        print("=== blocked decks ===")
        for deck, missing in sorted(partial):
            print(f"  {deck}: {', '.join(missing)}")

    if args.json_out:
        json.dump(
            {
                "fully_supported": len(full),
                "total": total,
                "blockers": impact,
                "blocked_decks": {d: m for d, m in sorted(partial)},
            },
            open(args.json_out, "w"),
            indent=2,
            sort_keys=True,
        )


if __name__ == "__main__":
    main()
