"""Prepare the reviewed Mutagen/Nifly fixture exclusions and source-build inputs."""

import hashlib
import json
from pathlib import Path
import sys
import zipfile

from source_filter import filter_zip, project_closure


def main():
    repo = Path(__file__).resolve().parents[2]
    sys.path.insert(0, str(repo / 'tools/repo_guard'))
    from byte_rules import check_signatures
    from path_rules import check_path
    manifest = repo / 'tools/licensing/sources-resolved.json'
    data = json.loads(manifest.read_text())
    output = repo / 'build/cache/filtered-sources'
    output.mkdir(parents=True, exist_ok=True)
    for entry in data['sources']:
        if not entry.get('unreviewedAssetPaths'):
            continue
        source = repo / entry['path']
        entry['excludedFiles'], entry['pathExceptions'] = [], []
        with zipfile.ZipFile(source) as archive:
            for name in entry['unreviewedAssetPaths']:
                content = archive.read(name)
                findings = check_path(name) + check_signatures(name, content)
                record = {'path': name, 'sha256': hashlib.sha256(content).hexdigest()}
                if name.endswith('.cs') and all(f.rule == 'forbidden-directory' for f in findings) and b'\x00' not in content:
                    entry['pathExceptions'].append({**record, 'rules': ['forbidden-directory'],
                        'reason': 'Pinned upstream C# source uses a source directory named Cache; not extracted game data.'})
                    continue
                test_paths = ('.UnitTests/', '/NiflySharp.Test/', '/Mutagen.Bethesda.Testing/Files/')
                if not any(marker in name for marker in test_paths):
                    raise ValueError(f'Exclusion outside reviewed test fixture directories: {name}')
                entry['excludedFiles'].append({**record,
                    'reason': 'Unneeded game-format test fixture; excluded from the production library source package.'})
            root = archive.namelist()[0].split('/')[0]
        if source.name.startswith('NiflySharp-'):
            submodule = repo / 'build/cache/legal-sources/nifxml-e18445870f177b374ecaf30e62af2a5f36e9fae7.zip'
            entry['additionalSources'] = [{
                'url': 'https://codeload.github.com/ousnius/nifxml/zip/e18445870f177b374ecaf30e62af2a5f36e9fae7',
                'path': submodule.relative_to(repo).as_posix(), 'sha256': hashlib.sha256(submodule.read_bytes()).hexdigest(),
                'stripPrefix': 'nifxml-e18445870f177b374ecaf30e62af2a5f36e9fae7', 'destination': root + '/nifxml'}]
            project = root + '/NiflySharp/NiflySharp.csproj'
        else:
            project = root + '/Mutagen.Bethesda/Mutagen.Bethesda.csproj'
        target = output / source.name
        if target.exists():
            if target.is_symlink():
                raise ValueError(f'Linked source output: {target}')
            target.unlink()
        filter_zip(source, target, entry, lambda path: repo / path)
        entry['preparedSha256'] = hashlib.sha256(target.read_bytes()).hexdigest()
        entry['projectReferenceClosure'] = project_closure(target, project)
        entry['buildValidation'] = 'Production project-reference and AdditionalFiles closure verified statically; targeted build remains required.'
        print(f'{source.name}: {len(entry["excludedFiles"])} excluded fixtures; {len(entry["projectReferenceClosure"])} production projects retained')
    manifest.write_text(json.dumps(data, indent=2) + '\n')


if __name__ == '__main__':
    main()
