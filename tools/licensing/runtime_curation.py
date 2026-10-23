"""Omit the explicitly excluded files from the downloaded runtime image."""
import argparse
import json
from pathlib import Path

from artifact_safety import digest, safe_name


def remove_excluded_files(runtime, destination):
    prefix = runtime['Destination'] + '/'
    root = destination.resolve()
    for entry in runtime['ExcludedFiles']:
        name = safe_name(entry['path'])
        if not name.startswith(prefix):
            raise ValueError('Runtime exclusion is outside its destination.')
        path = root / safe_name(name[len(prefix):])
        if not path.resolve().is_relative_to(root) or path.is_symlink():
            raise ValueError('Runtime exclusion traverses a link.')
        if path.exists():
            if digest(path) != entry['sha256']:
                raise ValueError(f'Excluded runtime file differs from the pinned version: {name}')
            path.unlink()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, required=True)
    parser.add_argument('--runtime', choices=('Python', 'Java'), required=True)
    parser.add_argument('--destination', type=Path, required=True)
    args = parser.parse_args()
    policy = json.loads((args.repo / 'tools/licensing/runtime-inputs.json').read_text())
    runtime = next(item for item in policy['runtimes'] if item['Name'] == args.runtime)
    remove_excluded_files(runtime, args.destination)
