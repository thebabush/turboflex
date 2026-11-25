# /// script
# requires-python = '>=3.11'
# ///
"""Turn a rust-analyzer SCIP index into the common edge TSV (module-level quality only).

Each non-definition, non-import reference to a workspace symbol becomes an edge from the
innermost enclosing definition in the same document to the referenced symbol. Module paths
come from the leading namespace (`name/`) descriptors of each symbol.

usage: uv run scip_edges.py <index.scip> <out.tsv> <workspace_crate> [<workspace_crate> ...]
"""

import json
import subprocess
import sys
from dataclasses import dataclass

ROLE_DEFINITION = 1
ROLE_IMPORT = 2


@dataclass(frozen=True)
class Sym:
    crate: str
    descriptors: list[str]

    @property
    def module(self) -> str:
        parts = [self.crate]
        for d in self.descriptors:
            if not d.endswith('/'):
                break
            name = d[:-1].strip('`')
            if name != 'crate':
                parts.append(name)
        return '::'.join(parts)

    @property
    def path(self) -> str:
        return '::'.join([self.crate] + [d.rstrip('/#.!').strip('`') for d in self.descriptors if d != 'crate/'])


def split_descriptors(s: str) -> list[str]:
    out: list[str] = []
    cur = ''
    depth = 0
    tick = False
    for ch in s:
        cur += ch
        if ch == '`':
            tick = not tick
        elif tick:
            continue
        elif ch == '[':
            depth += 1
        elif ch == ']':
            depth -= 1
        elif depth == 0 and ch in '/#!':
            out.append(cur)
            cur = ''
        elif depth == 0 and ch == '.' and not cur.endswith('().'):
            out.append(cur)
            cur = ''
        elif depth == 0 and cur.endswith('().'):
            out.append(cur)
            cur = ''
    if cur:
        out.append(cur)
    return out


def parse_symbol(symbol: str) -> Sym | None:
    if not symbol or symbol.startswith('local '):
        return None
    # '<scheme> <manager> <package> <version> <descriptors>'; version may be a URL for std.
    parts = symbol.split(' ', 4)
    if len(parts) < 5:
        return None
    return Sym(parts[2].replace('-', '_'), split_descriptors(parts[4]))


def contains(outer: list[int], line: int, col: int) -> bool:
    sl, sc = outer[0], outer[1]
    el, ec = (outer[2], outer[3]) if len(outer) == 4 else (outer[0], outer[2])
    return (sl, sc) <= (line, col) <= (el, ec)


def size(r: list[int]) -> tuple[int, int]:
    return (r[2] - r[0], r[3] - r[1]) if len(r) == 4 else (0, r[2] - r[1])


def main() -> None:
    index, out, *crates = sys.argv[1:]
    workspace = set(crates)
    data = json.loads(subprocess.run(['scip', 'print', '--json', index], check=True, capture_output=True).stdout)
    rows: set[tuple[str, ...]] = set()
    for doc in data['documents']:
        occs = doc.get('occurrences') or []
        defs = []
        for o in occs:
            sym = parse_symbol(o.get('symbol', ''))
            if sym and o.get('symbol_roles', 0) & ROLE_DEFINITION and o.get('enclosing_range'):
                defs.append((o['enclosing_range'], sym))
        for o in occs:
            roles = o.get('symbol_roles', 0)
            if roles & (ROLE_DEFINITION | ROLE_IMPORT):
                continue
            dst = parse_symbol(o.get('symbol', ''))
            if not dst or dst.crate not in workspace or dst.descriptors[-1:] and dst.descriptors[-1].endswith('/'):
                continue
            line, col = o['range'][0], o['range'][1]
            enclosing = [d for d in defs if contains(d[0], line, col)]
            if not enclosing:
                continue
            src = min(enclosing, key=lambda d: size(d[0]))[1]
            kind = 'macro_use' if dst.descriptors[-1].endswith('!') else 'ref'
            rows.add((src.path, src.module, dst.path, dst.module, kind, 'source', f'{doc["relative_path"]}:{line + 1}', ''))
    with open(out, 'w') as f:
        f.write('src_item\tsrc_module\tdst_item\tdst_module\tkind\tprovenance\tfile:line\tdetail\n')
        for r in sorted(rows):
            f.write('\t'.join(r) + '\n')
    print(f'{len(rows)} edges -> {out}')


if __name__ == '__main__':
    main()
