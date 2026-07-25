#!/usr/bin/env python3
"""Layout linter for THTML/TCSS.

`oxiterm check` validates parse structure. It does not look at layout, and TCSS
discards unrecognised properties without warning, so a file can pass `check`
while its styling silently does nothing. This script covers that gap.

Checks performed:

  E001  unknown TCSS property (silently dropped by the parser)
  E002  non-integer value for an integer property (parses to 0)
  E003  bordered element with height < 3 (borders alone need 2 rows)
  E004  bordered element with width < 3
  W005  rigid container whose children's heights exceed its inner space
  W006  ambiguous-width symbol inside a clickable label (clicks may miss)
  I007  ambiguous-width letter inside a clickable label (informational)

Usage:
    python lint_layout.py FILE.thtml [FILE2.thtml ...]
    python lint_layout.py path/to/dir          # walks *.thtml
    python lint_layout.py FILE.thtml --strict     # exit 1 also on W-level
    python lint_layout.py --list-properties

Exit status is 1 if any E-level finding was reported, else 0.
"""

from __future__ import annotations

import re
import sys
import unicodedata
import xml.etree.ElementTree as ET
from pathlib import Path

# ---------------------------------------------------------------------------
# Ground truth, mirrored from oxiterm-renderer/src/parser/tcss.rs
# ---------------------------------------------------------------------------

INT_PROPS = {
    "width", "height",
    "padding", "padding-top", "padding-right", "padding-bottom", "padding-left",
    "margin", "margin-top", "margin-right", "margin-bottom", "margin-left",
}

ENUM_PROPS = {
    "flex-direction": {"row", "column"},
    "align-items": {"flex-start", "flex-end", "center", "stretch"},
    "justify-content": {
        "flex-start", "flex-end", "center", "space-between", "space-around",
    },
    "wrap": {"word", "none"},
    "border-style": {"single", "double", "rounded"},
}

COLOR_PROPS = {"fg", "color", "bg", "background-color", "border", "border-color"}

FLOAT_PROPS = {"flex"}

KNOWN_PROPS = INT_PROPS | FLOAT_PROPS | set(ENUM_PROPS) | COLOR_PROPS

BORDER_PROPS = {"border", "border-color", "border-style"}

VALID_TAGS = {"screen", "box", "text", "input", "button", "img", "video", "for", "diagram"}

CLICKABLE_ATTRS = {"event-htmx"}


def is_ambiguous_width(ch: str) -> bool:
    """East Asian Ambiguous: one column in the model, possibly two on screen."""
    return unicodedata.east_asian_width(ch) == "A"


# ---------------------------------------------------------------------------
# Parsing
# ---------------------------------------------------------------------------

Finding = tuple[str, str, str]  # (code, location, message)


def parse_decls(text: str) -> list[tuple[str, str]]:
    """Split a TCSS declaration list into (property, value) pairs."""
    out = []
    for chunk in text.split(";"):
        if ":" not in chunk:
            continue
        prop, _, value = chunk.partition(":")
        prop = prop.strip().lower()
        if prop:
            out.append((prop, value.strip()))
    return out


def parse_stylesheet(css: str) -> dict[str, list[tuple[str, str]]]:
    """Map selector text -> declarations. Comments are stripped first."""
    css = re.sub(r"/\*.*?\*/", "", css, flags=re.S)
    rules: dict[str, list[tuple[str, str]]] = {}
    for match in re.finditer(r"([^{}]+)\{([^{}]*)\}", css):
        selector = match.group(1).strip()
        if not selector:
            continue
        rules.setdefault(selector, []).extend(parse_decls(match.group(2)))
    return rules


def resolve_style(
    elem: ET.Element, rules: dict[str, list[tuple[str, str]]]
) -> list[tuple[str, str]]:
    """Apply the cascade: tag, then class, then id, then inline."""
    decls: list[tuple[str, str]] = []
    tag = elem.tag.lower()
    decls += rules.get(tag, [])
    for cls in (elem.get("class") or "").split():
        decls += rules.get("." + cls, [])
    if elem.get("id"):
        decls += rules.get("#" + elem.get("id"), [])
    decls += parse_decls(elem.get("style") or "")
    return decls


def computed(decls: list[tuple[str, str]]) -> dict[str, str]:
    """Last declaration wins, matching the engine's single-phase cascade."""
    out: dict[str, str] = {}
    for prop, value in decls:
        out[prop] = value
    return out


def as_int(value: str) -> int | None:
    try:
        return int(value.strip())
    except ValueError:
        return None


# ---------------------------------------------------------------------------
# Checks
# ---------------------------------------------------------------------------

def check_declarations(where: str, decls: list[tuple[str, str]]) -> list[Finding]:
    findings = []
    for prop, value in decls:
        if prop not in KNOWN_PROPS:
            findings.append((
                "E001", where,
                f"unknown TCSS property '{prop}' — the parser discards this "
                f"silently, it has no effect",
            ))
            continue
        if prop in INT_PROPS and as_int(value) is None:
            findings.append((
                "E002", where,
                f"'{prop}: {value}' is not an integer — parses to 0"
                + (" (omit the property instead of writing 'auto')"
                   if value.strip().lower() == "auto" else ""),
            ))
        if prop in FLOAT_PROPS:
            try:
                val = float(value.strip())
                if val <= 0:
                    findings.append((
                        "E002", where,
                        f"'{prop}: {value}' must be greater than 0 — "
                        f"the parser discards this silently",
                    ))
            except ValueError:
                findings.append((
                    "E002", where,
                    f"'{prop}: {value}' is not a number — "
                    f"the parser discards this silently",
                ))
        if prop in ENUM_PROPS and value.strip().lower() not in ENUM_PROPS[prop]:
            findings.append((
                "E002", where,
                f"'{prop}: {value}' is not a recognised value "
                f"(allowed: {', '.join(sorted(ENUM_PROPS[prop]))}) — "
                f"silently falls back to the default",
            ))
    return findings


def describe(elem: ET.Element) -> str:
    bits = [elem.tag]
    if elem.get("id"):
        bits.append(f"#{elem.get('id')}")
    if elem.get("class"):
        bits.append("." + ".".join(elem.get("class").split()))
    return "<" + " ".join(bits) + ">"


def check_element(
    elem: ET.Element, rules: dict[str, list[tuple[str, str]]], path: str
) -> list[Finding]:
    findings: list[Finding] = []
    where = f"{path}: {describe(elem)}"

    if elem.tag.lower() not in VALID_TAGS:
        findings.append((
            "E001", where,
            f"unknown tag <{elem.tag}> — this is a hard parse error, "
            f"only {', '.join(sorted(VALID_TAGS))} exist",
        ))
        return findings

    decls = resolve_style(elem, rules)
    findings += check_declarations(where, decls)

    style = computed(decls)
    has_border = any(p in style for p in BORDER_PROPS)

    if has_border:
        h = as_int(style.get("height", ""))
        if h is not None and 0 < h < 3:
            findings.append((
                "E003", where,
                f"bordered element with height: {h} — the border alone needs "
                f"2 rows, leaving {h - 2} for content; minimum is 3",
            ))
        w = as_int(style.get("width", ""))
        if w is not None and 0 < w < 3:
            findings.append((
                "E004", where,
                f"bordered element with width: {w} — the border alone needs "
                f"2 columns; minimum is 3",
            ))

    # Vertical budget: only meaningful when the parent height is rigid and
    # every child height is known.
    parent_h = as_int(style.get("height", ""))
    direction = style.get("flex-direction", "row").strip().lower()
    children = [c for c in elem if c.tag.lower() in VALID_TAGS]
    if parent_h is not None and direction == "column" and children:
        pad = as_int(style.get("padding", "0")) or 0
        pad_top = as_int(style.get("padding-top", "")) 
        pad_bot = as_int(style.get("padding-bottom", ""))
        inner = parent_h
        inner -= 2 if has_border else 0
        inner -= (pad_top if pad_top is not None else pad)
        inner -= (pad_bot if pad_bot is not None else pad)

        total = 0
        all_known = True
        for child in children:
            cs = computed(resolve_style(child, rules))
            ch = as_int(cs.get("height", ""))
            if ch is None:
                all_known = False
                break
            m = as_int(cs.get("margin", "0")) or 0
            mt = as_int(cs.get("margin-top", ""))
            mb = as_int(cs.get("margin-bottom", ""))
            total += ch + (mt if mt is not None else m) + (mb if mb is not None else m)

        if all_known and total > inner:
            findings.append((
                "W005", where,
                f"children need {total} rows but only {inner} are available "
                f"(height {parent_h}"
                + (" minus 2 for the border" if has_border else "")
                + ") — children will overflow the frame",
            ))

    # Clickable labels must use unambiguous glyphs. Ambiguous *symbols* (arrows,
    # box-drawing, times sign) are a real hit-box risk and worth changing.
    # Ambiguous *letters* — ó ł ń Ł and friends — are unavoidable in Polish,
    # German, French text and render narrow in practice, so they are reported
    # separately rather than buried in the same bucket. Note that the engine's
    # own warn_ambiguous_clickables does not make this distinction, which is why
    # its output is very noisy for non-English UIs.
    if any(elem.get(a) for a in CLICKABLE_ATTRS):
        label = "".join(elem.itertext())
        amb = {c for c in label if is_ambiguous_width(c)}
        symbols = sorted(c for c in amb if not unicodedata.category(c).startswith("L"))
        letters = sorted(c for c in amb if unicodedata.category(c).startswith("L"))
        if symbols:
            findings.append((
                "W006", where,
                f"clickable label contains ambiguous-width symbol(s) "
                f"{' '.join(symbols)} — these may render 2 columns wide and "
                f"push the text off its hit box; use ASCII equivalents "
                f"('<' for '\u2190', '>' for '\u2192', 'x' for '\u00d7')",
            ))
        if letters:
            findings.append((
                "I007", where,
                f"clickable label contains ambiguous-width letter(s) "
                f"{' '.join(letters)} — normally harmless, but the engine will "
                f"also warn about these at load time",
            ))

    return findings


def lint_file(path: Path) -> list[Finding]:
    raw = path.read_text(encoding="utf-8")

    styles = re.findall(r"<style[^>]*>(.*?)</style>", raw, flags=re.S | re.I)
    rules = parse_stylesheet("\n".join(styles))

    findings: list[Finding] = []
    for selector, decls in rules.items():
        findings += check_declarations(f"{path}: {selector} {{...}}", decls)

    body = re.sub(r"<style[^>]*>.*?</style>", "", raw, flags=re.S | re.I)
    try:
        root = ET.fromstring(f"<screen>{body}</screen>")
    except ET.ParseError as exc:
        findings.append(("E000", str(path), f"not well-formed: {exc}"))
        return findings

    for elem in root.iter():
        if elem is root:
            continue
        findings += check_element(elem, rules, str(path))

    return findings


def main(argv: list[str]) -> int:
    args = argv[1:]
    if not args or args[0] in {"-h", "--help"}:
        print(__doc__)
        return 0
    if args[0] == "--list-properties":
        for prop in sorted(KNOWN_PROPS):
            print(prop)
        return 0

    strict = "--strict" in args
    args = [a for a in args if a != "--strict"]

    targets: list[Path] = []
    for arg in args:
        p = Path(arg)
        if p.is_dir():
            targets += sorted(p.rglob("*.thtml"))
        else:
            targets.append(p)

    all_findings: list[Finding] = []
    for path in targets:
        all_findings += lint_file(path)

    for code, where, message in all_findings:
        print(f"{code} {where}\n      {message}")

    errors = sum(1 for c, _, _ in all_findings if c.startswith("E"))
    warnings = sum(1 for c, _, _ in all_findings if c.startswith("W"))
    infos = len(all_findings) - errors - warnings
    print(f"\n{len(targets)} file(s): {errors} error(s), "
          f"{warnings} warning(s), {infos} info")
    if strict:
        return 1 if (errors or warnings) else 0
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
