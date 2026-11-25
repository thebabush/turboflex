#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Score extracted dependency edges against the `tricky` fixture's ground truth.

Usage:
    uv run scoring/score.py EXPECTED.tsv EXTRACTED [EXTRACTED ...]
        [--cfg test] [--cfg feature=extra] [--no-auto-cfg]
        [--workspace-crates app,core_api,macro_kit] [--absent SRC=>DST ...]

EXTRACTED is a TSV file, or a directory of turboflex fact files (`*.jsonl`, one per compile
unit, as written by crates/driver).

Extracted TSVs use the column schema
    src_item  src_module  dst_item  dst_module  kind  provenance  [file:line  detail ...]
with optional `#` comment lines and an optional `src_item...` header row. Several files are
unioned. Configs (`test`, `feature=X`) enable `[cfg:...]` rows; besides `--cfg`, they are read
from `# ... target=test features=a,b` comment lines unless `--no-auto-cfg`.

Fact files are merged like `turboflex_graph::Graph::merge`: edges are deduplicated by
(src, dst, kind), an edge seen only in `mode: test` units is test-only (`target=test` is appended
to its detail), and an endpoint's module is its entity's `module`. Edges to `external` entities
are non-workspace. Configs come from the unit headers (`mode: test`, `features`) unless
`--no-auto-cfg`.

Rules (from the fixture README):
- `[alt:G]` rows form one requirement; any member satisfies it, no member is a false positive.
- `[optional]` rows are not required and not false positives.
- `[cfg:...]` rows count only if that config was extracted; otherwise they are neutral.
- Ignored extracted rows (neither TP nor FP): `use`/re-export kinds, edges from an item inside an
  impl to that impl's own Self type or trait, non-workspace targets, same-module edges.
- The deliberately absent edges (default `app::callbacks::apply => app::math::triple`) are
  false positives and flagged as such.

Two levels: module pairs (src_module, dst_module) -- primary -- and item pairs with light
normalization (whitespace, lifetimes, generic args, std/alloc -> core) -- secondary.
"""

import argparse
import json
import re
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path

DEFAULT_ABSENT = ['app::callbacks::apply=>app::math::triple']
IGNORED_KINDS = {'use', 'reexport', 're_export'}


@dataclass(frozen=True)
class ExpectedRow:
    case: str
    src_item: str
    src_module: str
    dst_item: str
    dst_module: str
    kind: str
    provenance: str
    notes: str
    alt: str | None
    optional: bool
    cfg: str | None


@dataclass(frozen=True)
class ExtractedRow:
    src_item: str
    src_module: str
    dst_item: str
    dst_module: str
    kind: str
    provenance: str
    loc: str
    detail: str
    origin: str
    dst_external: bool = False


@dataclass
class Requirement:
    """One recall unit: a single expected row or an `[alt]` group of rows."""

    case: str
    label: str
    provenance: str
    rows: list[ExpectedRow]
    items: set[tuple[str, str]] = field(default_factory=set)
    modules: set[tuple[str, str]] = field(default_factory=set)


@dataclass
class LevelResult:
    hits: list[Requirement]
    misses: list[Requirement]
    true_pos: set[tuple[str, str]]
    false_pos: set[tuple[str, str]]
    neutral: set[tuple[str, str]]

    @property
    def recall(self) -> float:
        total = len(self.hits) + len(self.misses)
        return len(self.hits) / total if total else 0.0

    @property
    def precision(self) -> float:
        total = len(self.true_pos) + len(self.false_pos)
        return len(self.true_pos) / total if total else 0.0


# ---- normalization ------------------------------------------------------------------------------


def strip_generics(s: str) -> str:
    """Drop `<...>` argument lists that follow an identifier (`Foo<T>`, `impl<T: X>`), keeping
    qualified-path brackets (`<A as B>::m`)."""
    out: list[str] = []
    i = 0
    while i < len(s):
        c = s[i]
        if c == '<' and out and (out[-1].isalnum() or out[-1] == '_'):
            depth = 0
            while i < len(s):
                if s[i] == '<':
                    depth += 1
                elif s[i] == '>':
                    depth -= 1
                    if depth == 0:
                        break
                i += 1
            i += 1
            continue
        out.append(c)
        i += 1
    return ''.join(out)


def norm_item(s: str) -> str:
    s = re.sub(r"'[A-Za-z_]\w*\s*,?\s*", '', s)
    s = strip_generics(s)
    s = re.sub(r'\b(std|alloc)::', 'core::', s)
    return re.sub(r'\s+', ' ', s).strip()


def norm_module(s: str) -> str:
    return s.strip()


def crate_of(path: str) -> str:
    return path.split('::', 1)[0].lstrip('<')


# ---- loading ------------------------------------------------------------------------------------


def load_expected(path: Path) -> list[ExpectedRow]:
    rows: list[ExpectedRow] = []
    for line in path.read_text().splitlines():
        if not line.strip() or line.startswith('#') or line.startswith('case\t'):
            continue
        cols = line.split('\t') + [''] * 8
        notes = cols[7]
        alt = re.search(r'\[alt:([^\]]+)\]', notes)
        cfg = re.search(r'\[cfg:([^\]]+)\]', notes)
        rows.append(
            ExpectedRow(
                case=cols[0],
                src_item=cols[1],
                src_module=cols[2],
                dst_item=cols[3],
                dst_module=cols[4],
                kind=cols[5],
                provenance=cols[6],
                notes=notes,
                alt=alt.group(1) if alt else None,
                optional='[optional]' in notes,
                cfg=cfg.group(1) if cfg else None,
            )
        )
    return rows


def load_extracted(paths: list[Path]) -> tuple[list[ExtractedRow], set[str]]:
    """Rows of all files, plus configs announced in `# ... target=test features=a,b` lines."""
    rows: list[ExtractedRow] = []
    configs: set[str] = set()
    for path in paths:
        for line in path.read_text().splitlines():
            if line.startswith('#'):
                if re.search(r'\btarget=test\b', line):
                    configs.add('test')
                if m := re.search(r'\bfeatures=(\S*)', line):
                    configs.update(f'feature={f}' for f in m.group(1).split(',') if f)
                continue
            if not line.strip() or line.startswith('src_item\t'):
                continue
            cols = line.split('\t') + [''] * 8
            rows.append(ExtractedRow(*cols[:8], origin=path.name))
    return rows, configs


def load_facts(paths: list[Path]) -> tuple[list[ExtractedRow], set[str]]:
    """Edges of turboflex fact files, merged like `Graph::merge`, plus configs from unit headers."""
    entities: dict[str, dict] = {}
    # (src, dst, kind) -> (representative edge, seen only in test-mode units)
    merged: dict[tuple[str, str, str], tuple[dict, bool]] = {}
    configs: set[str] = set()
    for path in paths:
        test_mode = False
        for line in path.read_text().splitlines():
            record = json.loads(line)
            match record['record']:
                case 'header':
                    test_mode = record['mode'] == 'test'
                    if test_mode:
                        configs.add('test')
                    configs.update(f'feature={f}' for f in record['features'])
                case 'entity':
                    entities.setdefault(record['path'], record)
                case 'edge':
                    key = (record['src'], record['dst'], record['kind'])
                    kept = merged.get(key)
                    if kept is None or (kept[1] and not test_mode):
                        merged[key] = (record, test_mode)
                case other:
                    raise ValueError(f'{path}: unknown record {other!r}')
    rows: list[ExtractedRow] = []
    for edge, test_only in merged.values():
        span = edge['span']
        detail = edge['detail']
        if test_only:
            detail = f'{detail};target=test' if detail else 'target=test'
        dst = entities[edge['dst']]
        rows.append(
            ExtractedRow(
                src_item=edge['src'],
                src_module=entities[edge['src']]['module'],
                dst_item=edge['dst'],
                dst_module=dst['module'],
                kind=edge['kind'],
                provenance=edge['provenance'],
                loc=f'{span["file"]}:{span["line"]}' if span else '',
                detail=detail,
                origin='facts',
                dst_external=dst['origin'] == 'external',
            )
        )
    return rows, configs


def load_inputs(paths: list[Path]) -> tuple[list[ExtractedRow], set[str]]:
    """TSV files and fact-file directories, unioned."""
    tsvs = [p for p in paths if not p.is_dir()]
    rows, configs = load_extracted(tsvs)
    for directory in (p for p in paths if p.is_dir()):
        store = directory / '.turboflex'
        try:
            name = (store / 'current').read_text()
        except FileNotFoundError:
            pass
        else:
            if not name.startswith('facts-') or not name.isascii() or not name.replace('-', '').isalnum():
                raise ValueError(f'Invalid snapshot name in {store}')
            directory = store / name
        fact_rows, fact_configs = load_facts(sorted(directory.glob('*.jsonl')))
        rows += fact_rows
        configs |= fact_configs
    return rows, configs


# ---- scoring ------------------------------------------------------------------------------------


def impl_owner_parts(src_item: str) -> set[str]:
    """Self type and trait of the impl an item lives in (`<Self as Trait>::m`, or `Self::m`)."""
    if m := re.fullmatch(r'<(.+) as (.+)>::\w+', src_item):
        return {m.group(1), m.group(2)}
    if '::' in src_item:
        return {src_item.rsplit('::', 1)[0]}
    return set()


def ignore_reason(row: ExtractedRow, workspace: set[str]) -> str | None:
    if row.kind in IGNORED_KINDS:
        return 'use'
    if row.dst_external or crate_of(row.dst_module or row.dst_item) not in workspace:
        return 'non-workspace'
    if norm_module(row.src_module) == norm_module(row.dst_module):
        return 'same-module'
    src = norm_item(row.src_item)
    if not src.startswith('impl ') and norm_item(row.dst_item) in {norm_item(p) for p in impl_owner_parts(src)}:
        return 'impl-own-self-or-trait'
    return None


def build_requirements(rows: list[ExpectedRow]) -> list[Requirement]:
    reqs: list[Requirement] = []
    groups: dict[str, Requirement] = {}
    for row in rows:
        if row.optional:
            continue
        if row.alt:
            req = groups.get(row.alt)
            if req is None:
                req = Requirement(row.case, f'[alt:{row.alt}]', row.provenance, [])
                groups[row.alt] = req
                reqs.append(req)
        else:
            req = Requirement(row.case, f'{row.src_item} -> {row.dst_item}', row.provenance, [])
            reqs.append(req)
        req.rows.append(row)
        req.items.add((norm_item(row.src_item), norm_item(row.dst_item)))
        req.modules.add((norm_module(row.src_module), norm_module(row.dst_module)))
    return reqs


def score_level(
    reqs: list[Requirement],
    extracted: set[tuple[str, str]],
    allowed: set[tuple[str, str]],
    neutral: set[tuple[str, str]],
    key: str,
) -> LevelResult:
    hits: list[Requirement] = []
    misses: list[Requirement] = []
    required: set[tuple[str, str]] = set()
    for req in reqs:
        pairs = req.items if key == 'items' else req.modules
        required |= pairs
        (hits if pairs & extracted else misses).append(req)
    true_pos = extracted & required
    neutral_hits = (extracted & (allowed | neutral)) - true_pos
    false_pos = extracted - true_pos - neutral_hits
    return LevelResult(hits, misses, true_pos, false_pos, neutral_hits)


def module_level_reqs(reqs: list[Requirement]) -> list[Requirement]:
    """Module-level requirements: one per distinct set of alternative module pairs."""
    seen: dict[frozenset[tuple[str, str]], Requirement] = {}
    for req in reqs:
        key = frozenset(req.modules)
        if key not in seen:
            seen[key] = Requirement(req.case, ' | '.join(f'{s} -> {d}' for s, d in sorted(key)), req.provenance, [], modules=set(key))
        seen[key].rows.extend(req.rows)
    return list(seen.values())


def pct(x: float) -> str:
    return f'{100 * x:5.1f}%'


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('expected', type=Path)
    parser.add_argument('extracted', type=Path, nargs='+')
    parser.add_argument('--cfg', action='append', default=[], help='config the extractor ran: test, feature=NAME')
    parser.add_argument('--no-auto-cfg', action='store_true', help='ignore `# target=... features=...` lines')
    parser.add_argument('--workspace-crates', help='comma-separated; default: crates seen in the expected file')
    parser.add_argument('--absent', action='append', help='SRC=>DST item edge that must NOT be reported')
    parser.add_argument('--show-hits', action='store_true')
    args = parser.parse_args()

    expected = load_expected(args.expected)
    extracted_rows, auto_cfg = load_inputs(args.extracted)
    configs = set(args.cfg) | (set() if args.no_auto_cfg else auto_cfg)
    workspace = (
        set(args.workspace_crates.split(','))
        if args.workspace_crates
        else {crate_of(r.src_module) for r in expected} | {crate_of(r.dst_module) for r in expected}
    )
    absent = {tuple(norm_item(p) for p in a.split('=>')) for a in (args.absent or DEFAULT_ABSENT)}

    active = [r for r in expected if r.cfg is None or r.cfg in configs]
    inactive = [r for r in expected if r.cfg is not None and r.cfg not in configs]
    reqs = build_requirements(active)
    mod_reqs = module_level_reqs(reqs)

    # Allowed = any expected row (alt/optional); neutral = rows of configs that were not extracted.
    allowed_items = {(norm_item(r.src_item), norm_item(r.dst_item)) for r in active}
    allowed_mods = {(norm_module(r.src_module), norm_module(r.dst_module)) for r in active}
    neutral_items = {(norm_item(r.src_item), norm_item(r.dst_item)) for r in inactive} - allowed_items
    neutral_mods = {(norm_module(r.src_module), norm_module(r.dst_module)) for r in inactive} - allowed_mods

    kept: list[ExtractedRow] = []
    ignored: dict[str, int] = defaultdict(int)
    for row in extracted_rows:
        if reason := ignore_reason(row, workspace):
            ignored[reason] += 1
        else:
            kept.append(row)
    ext_items = {(norm_item(r.src_item), norm_item(r.dst_item)) for r in kept}
    ext_mods = {(norm_module(r.src_module), norm_module(r.dst_module)) for r in kept}
    # Absent edges are false positives even if some expected row happens to share the pair.
    allowed_items -= absent

    mod_res = score_level(mod_reqs, ext_mods, allowed_mods, neutral_mods, 'modules')
    item_res = score_level(reqs, ext_items, allowed_items, neutral_items, 'items')

    print(f'expected: {args.expected}  ({len(expected)} rows, {len(active)} active)')
    print(f'extracted: {", ".join(str(p) for p in args.extracted)}  ({len(extracted_rows)} rows)')
    print(f'configs: {", ".join(sorted(configs)) or "(default only)"}; workspace crates: {", ".join(sorted(workspace))}')
    print(f'ignored extracted rows: {dict(sorted(ignored.items()))}; {len(inactive)} cfg rows inactive')
    print()

    # Per-case table
    cases = sorted({r.case for r in expected})
    module_of_case = defaultdict(set)
    for r in expected:
        module_of_case[r.case].add(norm_module(r.src_module))
    print('per case            module-level         item-level           extras(item)')
    print('case  hit/total     misses               hit/total            ')
    fp_by_case: dict[str, list[ExtractedRow]] = defaultdict(list)
    fp_rows = [r for r in kept if (norm_item(r.src_item), norm_item(r.dst_item)) in item_res.false_pos]
    for r in fp_rows:
        owner = next((c for c in cases if norm_module(r.src_module) in module_of_case[c]), 'other')
        fp_by_case[owner].append(r)
    for case in cases:
        m_h = [q for q in mod_res.hits if q.case == case]
        m_m = [q for q in mod_res.misses if q.case == case]
        i_h = [q for q in item_res.hits if q.case == case]
        i_m = [q for q in item_res.misses if q.case == case]
        cfg_note = ' (cfg rows inactive)' if any(r.case == case for r in inactive) else ''
        print(
            f'{case}   {len(m_h):2}/{len(m_h) + len(m_m):<2}         {len(m_m):2} miss             '
            f'{len(i_h):2}/{len(i_h) + len(i_m):<2}                {len(fp_by_case.get(case, []))}{cfg_note}'
        )
    print()

    for name, res in (('MODULE-LEVEL (primary)', mod_res), ('ITEM-LEVEL (secondary)', item_res)):
        print(f'== {name}')
        print(
            f'recall    {pct(res.recall)}  ({len(res.hits)}/{len(res.hits) + len(res.misses)} requirements)\n'
            f'precision {pct(res.precision)}  ({len(res.true_pos)} TP pairs, {len(res.false_pos)} FP pairs, '
            f'{len(res.neutral)} neutral [optional/alt-only/inactive cfg])'
        )
        by_prov: dict[str, list[int]] = defaultdict(lambda: [0, 0])
        for q in res.hits:
            by_prov[q.provenance][0] += 1
            by_prov[q.provenance][1] += 1
        for q in res.misses:
            by_prov[q.provenance][1] += 1
        print('recall by expected provenance: ' + ', '.join(f'{p} {h}/{t}' for p, (h, t) in sorted(by_prov.items())))
        if res.misses:
            print('misses:')
            for q in res.misses:
                print(f'  {q.case} [{q.provenance}] {q.label}')
                if q.label.startswith('[alt'):
                    for r in q.rows:
                        print(f'        alt: {r.src_item} -> {r.dst_item}')
        if args.show_hits:
            print('hits:')
            for q in res.hits:
                print(f'  {q.case} {q.label}')
        if name.startswith('MODULE') and res.false_pos:
            print('false-positive module pairs:')
            for s, d in sorted(res.false_pos):
                print(f'  {s} -> {d}')
        print()

    print('== absent-edge check')
    for s, d in sorted(absent):
        hit = (s, d) in ext_items
        print(f'  {s} -> {d}: {"REPORTED (false positive)" if hit else "not reported (ok)"}')
    print()

    print('== item-level extras for manual triage (workspace targets, cross-module, not expected)')
    for case in [*cases, 'other']:
        rows = fp_by_case.get(case)
        if not rows:
            continue
        print(f'  [{case}]')
        seen: set[tuple[str, str]] = set()
        for r in sorted(rows, key=lambda r: (r.src_item, r.dst_item)):
            key = (norm_item(r.src_item), norm_item(r.dst_item))
            if key in seen:
                continue
            seen.add(key)
            tag = ' ABSENT-EDGE' if key in absent else ''
            print(f'    {r.src_item} -> {r.dst_item}  ({r.kind}, {r.provenance}, {r.loc}){tag}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
