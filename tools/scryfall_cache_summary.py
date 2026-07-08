#!/usr/bin/env python3
"""Summarize the local Scryfall bulk-data cache without loading it all at once."""

from __future__ import annotations

import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Iterator


ROOT = Path(__file__).resolve().parents[1]
CACHE_DIR = ROOT / "cards" / "scryfall"
REPORT = ROOT / "reports" / "gates" / "CP-LAYERS" / "scryfall-cache-2026-07-07.md"
METRICS = ROOT / "metrics" / "scryfall_cache_summary.json"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def iter_bulk_cards(path: Path) -> Iterator[dict[str, object]]:
    with path.open("r", encoding="utf-8") as handle:
        for raw in handle:
            line = raw.strip()
            if not line or line in {"[", "]"}:
                continue
            if line.endswith(","):
                line = line[:-1]
            if not line.startswith("{"):
                continue
            card = json.loads(line)
            if card.get("object") == "card":
                yield card


def find_all_cards_cache() -> Path:
    candidates = sorted(CACHE_DIR.glob("all-cards-*.json"))
    if not candidates:
        raise SystemExit(f"missing all-cards cache under {CACHE_DIR}")
    return candidates[-1]


def load_bulk_manifest() -> dict[str, object]:
    path = CACHE_DIR / "bulk-data.json"
    if not path.exists():
        raise SystemExit(f"missing Scryfall bulk manifest at {path}")
    return json.loads(path.read_text(encoding="utf-8"))


def bulk_entry(manifest: dict[str, object], bulk_type: str) -> dict[str, object]:
    for entry in manifest.get("data", []):
        if isinstance(entry, dict) and entry.get("type") == bulk_type:
            return entry
    raise SystemExit(f"Scryfall bulk manifest has no {bulk_type!r} entry")


def summarize_cards(path: Path) -> dict[str, object]:
    languages: Counter[str] = Counter()
    layouts: Counter[str] = Counter()
    legalities: Counter[str] = Counter()
    oracle_ids: set[str] = set()
    names: set[str] = set()
    total = 0
    english = 0

    for card in iter_bulk_cards(path):
        total += 1
        lang = str(card.get("lang") or "unknown")
        languages[lang] += 1
        layouts[str(card.get("layout") or "unknown")] += 1
        if lang == "en":
            english += 1
            name = card.get("name")
            oracle_id = card.get("oracle_id")
            if isinstance(name, str):
                names.add(name)
            if isinstance(oracle_id, str):
                oracle_ids.add(oracle_id)
            card_legalities = card.get("legalities")
            if isinstance(card_legalities, dict):
                for status in card_legalities.values():
                    legalities[str(status)] += 1

    return {
        "total_records": total,
        "english_records": english,
        "unique_english_names": len(names),
        "unique_english_oracle_ids": len(oracle_ids),
        "language_counts": dict(languages.most_common()),
        "layout_counts": dict(layouts.most_common()),
        "english_legality_value_counts": dict(legalities.most_common()),
    }


def write_outputs(
    cache_path: Path,
    manifest_entry: dict[str, object],
    manifest_hash: str,
    cache_hash: str,
    summary: dict[str, object],
) -> None:
    METRICS.parent.mkdir(parents=True, exist_ok=True)
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "source": "Scryfall all_cards bulk data",
        "download_uri": manifest_entry.get("download_uri"),
        "source_updated_at": manifest_entry.get("updated_at"),
        "local_cache": cache_path.relative_to(ROOT).as_posix(),
        "local_size_bytes": cache_path.stat().st_size,
        "local_sha256": cache_hash,
        "manifest_sha256": manifest_hash,
        **summary,
    }
    METRICS.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    language_rows = [
        f"| {language} | {count} |"
        for language, count in list(summary["language_counts"].items())[:12]
    ]
    layout_rows = [
        f"| {layout} | {count} |" for layout, count in list(summary["layout_counts"].items())[:12]
    ]
    lines = [
        "# Scryfall Cache Evidence",
        "",
        "Date: 2026-07-07",
        "",
        "Purpose: local card-data cache for CP-LAYERS Option 1 remediation.",
        "",
        "Policy posture: metadata/oracle text only. No card art, set symbols, official",
        "fonts, or image payloads were downloaded or committed.",
        "",
        "## Source",
        "",
        f"- Bulk type: `all_cards`",
        f"- Source updated at: `{manifest_entry.get('updated_at')}`",
        f"- Download URI: `{manifest_entry.get('download_uri')}`",
        f"- Local cache: `{cache_path.relative_to(ROOT).as_posix()}`",
        f"- Local size bytes: `{cache_path.stat().st_size}`",
        f"- Manifest SHA-256: `{manifest_hash}`",
        f"- All-cards SHA-256: `{cache_hash}`",
        "",
        "## Counts",
        "",
        f"- Total card records: `{summary['total_records']}`",
        f"- English card records: `{summary['english_records']}`",
        f"- Unique English names: `{summary['unique_english_names']}`",
        f"- Unique English oracle IDs: `{summary['unique_english_oracle_ids']}`",
        "",
        "## Top Languages",
        "",
        "| Language | Records |",
        "| --- | ---: |",
        *language_rows,
        "",
        "## Top Layouts",
        "",
        "| Layout | Records |",
        "| --- | ---: |",
        *layout_rows,
        "",
        "## Gate Note",
        "",
        "This cache helps the all-card representation effort and later importer work.",
        "It does not by itself satisfy the CP-LAYERS legacy engine differential; that",
        "still requires a runnable legacy Forge harness plus Forge 2.0 card-script",
        "translation for the selected 100-card layered subset.",
    ]
    REPORT.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> None:
    manifest_path = CACHE_DIR / "bulk-data.json"
    cache_path = find_all_cards_cache()
    manifest = load_bulk_manifest()
    entry = bulk_entry(manifest, "all_cards")
    summary = summarize_cards(cache_path)
    write_outputs(cache_path, entry, sha256(manifest_path), sha256(cache_path), summary)
    print(f"WROTE {METRICS}")
    print(f"WROTE {REPORT}")


if __name__ == "__main__":
    main()
