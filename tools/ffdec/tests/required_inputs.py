# Copyright 2026 Julian Thomas and OpenFO4 contributors
# SPDX-License-Identifier: Apache-2.0
"""Parent-scheduled comparison of the installer's real FFDec export jobs."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import re
import subprocess

from normalization import normalized


def run(java, jar, options, output, input_path, log, timeout):
    result = subprocess.run([str(java), "-Djava.awt.headless=true", "-Xmx1024m", "-jar", str(jar),
                             *options, str(output), str(input_path)], capture_output=True, text=True, timeout=timeout)
    log.write_text(result.stdout + "\nSTDERR:\n" + result.stderr, encoding="utf-8")
    if result.returncode:
        raise RuntimeError(f"Export failed (exit {result.returncode}): {input_path}; see {log}")


def outputs(directory):
    return {path.relative_to(directory).as_posix(): normalized(path)
            for path in directory.rglob('*') if path.is_file()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--plan', type=Path, required=True)
    parser.add_argument('--java', type=Path, required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    parser.add_argument('--reference', type=Path, required=True)
    parser.add_argument('--timeout', type=int, default=360)
    parser.add_argument('--candidate-name', default='candidate')
    args = parser.parse_args()
    plan = json.loads(args.plan.read_text(encoding='utf-8-sig'))
    root = args.plan.resolve().parent
    if not re.fullmatch(r'[A-Za-z0-9_-]+', args.candidate_name) or args.candidate_name in ('inputs', 'reference', 'logs'):
        raise ValueError('Unsafe candidate output directory')
    logs = root / 'logs'
    logs.mkdir(exist_ok=True)

    def job(record):
        name = record['id']
        if not re.fullmatch(r'[A-Za-z0-9_-]+', name):
            raise ValueError('Unsafe export job directory')
        input_path = (root / record['input']).resolve()
        if not input_path.is_relative_to(root):
            raise ValueError('Export input is outside the plan directory')
        reference, candidate = root / 'reference' / name, root / args.candidate_name / name
        if reference.exists() or candidate.exists():
            raise ValueError(f'Use fresh output directories for export comparison: {name}')
        for kind, jar, destination in (('reference', args.reference, reference), ('candidate', args.candidate, candidate)):
            destination.parent.mkdir(parents=True, exist_ok=True)
            run(args.java, jar, record['options'], destination, input_path, logs / f'{name}-{kind}.log', args.timeout)
        left, right = outputs(reference), outputs(candidate)
        if not left or left != right:
            raise AssertionError(f'Export output bytes/layout differ: {name}')
        if record.get('expectedFiles') is not None and sum(path.endswith('.svg') for path in right) != record['expectedFiles']:
            raise AssertionError(f'Export SVG frame count differs: {name}')
        print(f'PASS {name}: {len(right)} output files', flush=True)
        return {'id': name, 'files': len(right)}

    with ThreadPoolExecutor(max_workers=8) as pool:
        results = list(pool.map(job, plan['jobs']))
    (root / f'{args.candidate_name}-results.json').write_text(json.dumps(results, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
