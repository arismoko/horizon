"""Summarize completed campaigns using the exact main PID recorded by the perf wrapper."""
import argparse
import csv
import json
from pathlib import Path
import statistics

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('result', type=Path)
parser.add_argument('--variants', help='Read only these comma-separated variants, for a gated analysis')
args = parser.parse_args()
root = args.result
selected_variants = set(args.variants.split(',')) if args.variants else None
pids = {}
for line in (root / 'perf-invocations.txt').read_text().splitlines():
    args = line.split('\t')
    if '-p' in args and '-o' in args:
        name = Path(args[args.index('-o') + 1]).parent.name
        pids[name] = args[args.index('-p') + 1]
values = {}
for path in sorted((root / 'runs').rglob('perf.csv')):
    name = path.parent.name
    _, variant, round_number = name.split('-')
    if selected_variants is not None and variant not in selected_variants:
        continue
    ticks = [int(line) for line in (path.parent / 'ticks.txt').read_text().splitlines() if not line.startswith('#')]
    assert len(ticks) == 2000, (name, len(ticks))
    rows = list(csv.reader(path.read_text().splitlines()))
    selected = [row for row in rows if len(row) > 5 and row[0] == 'horizon-client-' + pids[name] and row[3] == 'instructions:u']
    assert len(selected) == 1, (name, selected)
    row = selected[0]
    assert float(row[5]) == 100.0, (name, 'counter multiplexing', row)
    value = float(row[1]) / len(ticks)
    values.setdefault(variant, {})[int(round_number)] = value
summary = {}
for variant, rounds in sorted(values.items()):
    runs = [v for _, v in sorted(rounds.items())]
    median = statistics.median(runs)
    summary[variant] = dict(runs=runs, n=len(runs), mean=statistics.mean(runs), median=median, min=min(runs), max=max(runs), spread_percent=100*(max(runs)-min(runs))/median)
assert all(item['n'] == 5 for item in summary.values()), summary
baseline_name = 'A' if 'A' in summary else 'base'
baseline = summary[baseline_name]
for item in summary.values():
    item[f'median_over_{baseline_name}_percent'] = 100*(item['median']/baseline['median']-1)
    item[f'mean_over_{baseline_name}_percent'] = 100*(item['mean']/baseline['mean']-1)
filename = 'instructions-summary-selected.json' if selected_variants is not None else 'instructions-summary.json'
(root / filename).write_text(json.dumps(summary, indent=2)+'\n')
print(f'| Variant | Mean M instructions/tick | Median M | Min M | Max M | Spread | Median vs {baseline_name} |')
print('| --- | ---: | ---: | ---: | ---: | ---: | ---: |')
for variant, item in summary.items():
    print(f"| {variant} | {item['mean']/1e6:.6f} | {item['median']/1e6:.6f} | {item['min']/1e6:.6f} | {item['max']/1e6:.6f} | {item['spread_percent']:.2f}% | {item[f'median_over_{baseline_name}_percent']:+.2f}% |")
