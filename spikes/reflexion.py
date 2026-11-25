# /// script
# requires-python = '>=3.11'
# ///
"""Spike: compute a reflexion model from an extracted edge TSV and an arch TOML.

usage: uv run spikes/reflexion.py <model.toml> <edges.tsv> <out.json> [--with-tests]

Throwaway code: it exists to see whether reflexion output on a real codebase is useful
before building the real engine.
"""

import json
import sys
import tomllib
from collections import Counter, defaultdict
from dataclasses import dataclass, field

SAMPLES = 6


@dataclass
class Pattern:
    component: str
    path: str
    glob: bool
    item: bool

    def matches(self, path: str) -> bool:
        if self.glob:
            return path.startswith(self.path + '::')
        if self.item:
            return path == self.path or path.startswith(self.path + '::')
        return path == self.path

    @property
    def rank(self) -> tuple[int, int, int]:
        # item patterns beat module patterns; then longer paths; then exact over glob
        return (int(self.item), self.path.count('::') + 1, int(not self.glob))


@dataclass
class Pair:
    edges: int = 0
    kinds: Counter = field(default_factory=Counter)
    provenance: Counter = field(default_factory=Counter)
    samples: list = field(default_factory=list)


def load_patterns(model: dict) -> list[Pattern]:
    pats = []
    for name, comp in model['components'].items():
        for m in comp.get('modules', []):
            glob = m.endswith('::**')
            pats.append(Pattern(name, m.removesuffix('::**'), glob, item=False))
        for it in comp.get('items', []):
            pats.append(Pattern(name, it, glob=False, item=True))
    return pats


def component_of(pats: list[Pattern], item: str, module: str) -> str | None:
    best = None
    for p in pats:
        if (p.item and p.matches(item)) or (not p.item and p.matches(module)):
            if best is None or p.rank > best.rank:
                best = p
    return best.component if best else None


def is_test(row: list[str]) -> bool:
    return 'target=test' in row[7] or any('::tests' in m or 'test_support' in m for m in (row[1], row[3]))


def main() -> None:
    args = [a for a in sys.argv[1:] if not a.startswith('--')]
    with_tests = '--with-tests' in sys.argv
    model_path, edges_path, out_path = args
    with open(model_path, 'rb') as f:
        model = tomllib.load(f)
    pats = load_patterns(model)
    crates = {p.path.split('::')[0] for p in pats}
    foundation = set(model.get('foundation', []))
    rules = {k: set(v) for k, v in model['rules'].items()}

    pairs: dict[tuple[str, str], Pair] = defaultdict(Pair)
    unmapped: Counter = Counter()
    skipped = Counter()
    for line in open(edges_path):
        if line.startswith(('#', 'src_item\t')):
            continue
        row = line.rstrip('\n').split('\t')
        if len(row) < 8:
            continue
        if row[1].split('::')[0] not in crates:
            skipped['non-model crate (integration tests, examples, bins)'] += 1
            continue
        if not with_tests and is_test(row):
            skipped['test code'] += 1
            continue
        src = component_of(pats, row[0], row[1])
        dst = component_of(pats, row[2], row[3])
        for comp, mod in ((src, row[1]), (dst, row[3])):
            if comp is None:
                unmapped[mod] += 1
        if src is None or dst is None or src == dst:
            continue
        pr = pairs[(src, dst)]
        pr.edges += 1
        pr.kinds[row[4]] += 1
        pr.provenance[row[5]] += 1
        if len(pr.samples) < SAMPLES or (row[5] == 'source' and any(s['provenance'] != 'source' for s in pr.samples)):
            sample = {'src': row[0], 'dst': row[2], 'kind': row[4], 'provenance': row[5], 'at': row[6], 'detail': row[7]}
            if len(pr.samples) >= SAMPLES:
                pr.samples = [s for s in pr.samples if s['provenance'] == 'source'] + [sample]
                pr.samples = pr.samples[:SAMPLES]
            else:
                pr.samples.append(sample)

    result = {'convergences': [], 'divergences': [], 'absences': []}
    for (a, b), pr in sorted(pairs.items(), key=lambda kv: -kv[1].edges):
        entry = {'from': a, 'to': b, 'edges': pr.edges, 'kinds': dict(pr.kinds), 'provenance': dict(pr.provenance), 'samples': pr.samples}
        allowed = b in rules.get(a, set()) or b in foundation
        result['convergences' if allowed else 'divergences'].append(entry)
    for a, allowed in rules.items():
        for b in sorted(allowed):
            if (a, b) not in pairs:
                result['absences'].append({'from': a, 'to': b})
    result['unmapped'] = [{'module': m, 'edges': c} for m, c in unmapped.most_common()]
    result['skipped'] = dict(skipped)
    result['components'] = {k: v.get('doc', '') for k, v in model['components'].items()}
    result['rules'] = {k: sorted(v) for k, v in rules.items()}
    result['foundation'] = sorted(foundation)
    with open(out_path, 'w') as f:
        json.dump(result, f, indent=1)

    print(f"convergences {len(result['convergences'])}, divergences {len(result['divergences'])}, "
          f"absences {len(result['absences'])}, unmapped modules {len(result['unmapped'])}; skipped {dict(skipped)}")
    print('\nDIVERGENCES')
    for d in result['divergences']:
        prov = ','.join(f'{k}:{v}' for k, v in d['provenance'].items())
        print(f"  {d['from']:>11} -> {d['to']:<10} {d['edges']:5} edges  [{prov}]")
        for s in d['samples'][:3]:
            print(f"      {s['src'][:70]} -> {s['dst'][:70]}  ({s['kind']}, {s['at']})")
    print('\nABSENCES')
    for a in result['absences']:
        print(f"  {a['from']} -> {a['to']}")
    print('\nUNMAPPED')
    for u in result['unmapped'][:20]:
        print(f"  {u['module']} ({u['edges']})")


if __name__ == '__main__':
    main()
