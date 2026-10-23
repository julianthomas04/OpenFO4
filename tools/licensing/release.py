"""Build the release's notices, matching sources and small artifact-pair manifest."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import zipfile

from artifact_safety import digest, safe_name, tree_files
from acquire import acquire_sources
from source_filter import filter_archive
from source_curation import enrich as enrich_sources
from producer_sources import prepare_ba2_source, prepare_ffdec_source


class ReleaseError(ValueError):
    pass


def load(path):
    return json.loads(Path(path).read_text(encoding='utf-8-sig'))


def write(path, value):
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    Path(path).write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')


def git(repo, *args):
    return subprocess.check_output(['git', '-C', str(repo), *args], text=True).strip()


def resolve_input(repo, value):
    roots = {'{nuget}': Path.home() / '.nuget/packages'}
    for token, root in roots.items():
        if value.startswith(token + '/'):
            return root / safe_name(value[len(token) + 1:])
    for token, subdir in (('{cargo}', 'src'), ('{cargo-cache}', 'cache')):
        if value.startswith(token + '/'):
            relative = safe_name(value[len(token) + 1:])
            registry = Path.home() / '.cargo/registry' / subdir
            candidates = [p / relative for p in registry.iterdir() if p.is_dir() and (p / relative).is_file()] if registry.is_dir() else []
            if not candidates and token == '{cargo-cache}':
                return repo / 'build/cache/legal-sources/cargo' / relative
            if len(candidates) != 1:
                raise ReleaseError(f'Expected one cached Cargo input: {value}; found {len(candidates)}')
            return candidates[0]
    candidate = Path(value)
    if candidate.is_absolute():
        raise ReleaseError(f'Nonportable input path: {value}')
    return repo / safe_name(value)


def ledgers(repo):
    paths = sorted((repo / 'tools/licensing').glob('*components.json'))
    packages = repo / 'tools/licensing/reviewed-packages.json'
    if packages.exists():
        paths.append(packages)
    if not paths:
        raise ReleaseError('No reviewed component ledgers exist.')
    components, ids = [], set()
    for path in paths:
        data = load(path)
        if data.get('schema') != 1:
            raise ReleaseError(f'Unsupported component ledger: {path}')
        for component in data['components']:
            if component['id'] in ids:
                raise ReleaseError(f'Duplicate component: {component["id"]}')
            ids.add(component['id'])
            components.append(component)
    return components


def package_components(repo):
    cargo = subprocess.check_output(['cargo', 'tree', '--locked', '--target', 'x86_64-pc-windows-msvc',
                                     '--edges', 'normal,build', '--prefix', 'none', '--format', '{p}'],
                                    cwd=repo / 'src/engine', text=True)
    import re
    cargo_ids = {f'cargo:{name}/{version}' for name, version in re.findall(r'^([^\s]+) v([^\s]+)', cargo, re.M)}
    nuget_ids = set()
    for project, program in (('src/installer', 'installer'), ('tools/bink2-decoder', 'Bink2Decoder')):
        path = repo / project / f'bin/Release/net8.0/win-x64/{program}.deps.json'
        if not path.is_file():
            raise ReleaseError(f'Build the Release win-x64 project before packaging its dependencies: {project}')
        data = load(path)
        for name, package in data['targets'][data['runtimeTarget']['name']].items():
            if data['libraries'][name]['type'] == 'package' and any(package.get(kind) for kind in ('runtime', 'native', 'resources', 'runtimeTargets')):
                nuget_ids.add('nuget:' + name.lower())
    components = []
    for component in ledgers(repo):
        ident = component['id']
        if ident.startswith('cargo:'):
            included = ident in cargo_ids
        elif ident.startswith('nuget:'):
            included = ident.lower() in nuget_ids
        else:
            included = component.get('role') == 'runtime'
        if included:
            components.append(dict(component, role='runtime'))
    known = {component['id'].lower() for component in components}
    missing = {name for name in cargo_ids | nuget_ids if name.lower() not in known
               and not name.startswith(('cargo:openfo4/', 'nuget:microsoft.netcore.app.'))}
    if missing:
        raise ReleaseError('Add license/source inventory for dependencies: ' + ', '.join(sorted(missing)))
    return enrich_sources(repo, components)


def prepare(repo):
    components = package_components(repo)
    baseline = load(repo / 'tools/licensing/ba2-source-inputs.json')['baseline']
    acquire_sources(repo, [{'role': 'runtime', 'sourceInputs': [baseline], 'licenseFiles': []}], resolve_input)
    prepare_ba2_source(repo)
    for component in components:
        if component['id'] == 'native:ffdec-suite':
            source = prepare_ffdec_source(repo)
            component['sourceInputs'] = [{'path': source.relative_to(repo).as_posix(), 'sha256': digest(source)}]
        for kind in ('licenseFiles', 'sourceInputs'):
            component[kind] = [entry for entry in component.get(kind, [])
                               if not entry['path'].startswith('tools/licensing/evidence/')]
    acquire_sources(repo, components, resolve_input)
    write(repo / 'build/release-components.json', {'schema': 1, 'components': components})
    return components


def copy_evidence(repo, components, source_root, payload):
    fields = ('id', 'version', 'licenseExpression', 'origin', 'modifications', 'sourceRequired', 'sourceMemberLicenses')
    records = [{key: component[key] for key in fields if key in component} for component in components if component['role'] == 'runtime']
    runtime_components = [component for component in components if component['role'] == 'runtime']
    for component, staged in zip(runtime_components, records, strict=True):
        dirname = component['id'].replace(':', '_').replace('/', '_')
        safe_name(dirname)
        for kind, base in (('licenseFiles', payload / 'legal/components' / dirname),
                           ('sourceInputs', source_root / 'components' / dirname)):
            output = []
            for number, entry in enumerate(component[kind]):
                source = resolve_input(repo, entry['path'])
                target = base / f'{number:03d}-{entry.get("preparedName", source.name)}'
                if kind == 'sourceInputs' and not entry.get('preparedPath') and (entry.get('excludedFiles') or entry.get('materializedLinks')):
                    target = target.with_name(target.name + '.preferred.zip')
                target.parent.mkdir(parents=True, exist_ok=True)
                if kind == 'sourceInputs' and entry.get('preparedPath'):
                    prepared = resolve_input(repo, entry['preparedPath'])
                    if digest(prepared) != entry['preparedSha256']:
                        raise ReleaseError('Prepared corresponding source bytes changed.')
                    shutil.copyfile(prepared, target)
                elif kind == 'sourceInputs' and (entry.get('excludedFiles') or entry.get('materializedLinks')):
                    recipe = {key: value for key, value in entry.items() if key != 'preparedSha256'}
                    filter_archive(source, target, recipe, lambda path: resolve_input(repo, path))
                else:
                    shutil.copyfile(source, target)
                root = payload if kind == 'licenseFiles' else source_root
                output.append({'path': target.relative_to(root).as_posix(), 'origin': entry.get('url', entry['path'])})
            staged[kind] = output
    return records


def stage(repo, payload, sources, version):
    components = load(repo / 'build/release-components.json')['components']
    if sources.exists():
        raise ReleaseError(f'Use a fresh source staging directory: {sources}')
    sources.mkdir(parents=True)
    archive = sources / 'OpenFO4-preferred-source.zip'
    subprocess.run(['git', '-C', str(repo), 'archive', '--format=zip', f'--output={archive}', 'HEAD'], check=True)
    records = copy_evidence(repo, components, sources, payload)
    settings = release_links(repo, version)
    write(sources / 'components.json', {'schema': 1, 'components': records})
    write(payload / 'legal/components.json', {'schema': 1, 'components': records})
    (payload / 'legal/COMPONENT_INDEX.txt').write_bytes(component_index(records, settings, version).encode('utf-8'))


def component_index(records, settings, version):
    notices = ['OpenFO4 third-party component index', '',
               f'Matching source asset: OpenFO4-{version}-sources.zip',
               'Source download: ' + settings['sourceUrl'], '',
               'Original OpenFO4 source: Apache-2.0. Combined installer: GPL-3.0.',
               'Imported components retain their own licenses.', '']
    for component in records:
        notices.extend([f'{component["id"]}: {component["licenseExpression"]}',
                        '  Origin: ' + json.dumps(component['origin'], ensure_ascii=False),
                        '  Modifications: ' + component['modifications'],
                        *[f'  Source-only portion: {part["member"]}: {part["licenseExpression"]}' for part in component.get('sourceMemberLicenses', [])],
                        '  License texts: ' + ', '.join(entry['path'] for entry in component['licenseFiles']),
                        '  Matching sources: ' + ', '.join(entry['path'] for entry in component['sourceInputs']), ''])
    return '\n'.join(notices)



def release_links(repo, version):
    settings = load(repo / 'tools/licensing/release-settings.json')
    repository = settings['repositoryUrl'].rstrip('/')
    tag = settings['releaseTagPattern'].format(version=version)
    placeholder = '{{' in repository or not repository.startswith('https://github.com/')
    return {'repositoryUrl': repository, 'sourceUrl': f'{repository}/releases/download/{tag}/OpenFO4-{version}-sources.zip',
            'placeholderLinks': placeholder,
            'publicSourceAccessVerified': False,
            'publicationReady': False,
            'publicationBlockers': ['Repository URL is a placeholder.'] if placeholder else ['Public source asset access has not been verified.']}


def file_record(root, path):
    return {'path': path.relative_to(root).as_posix(), 'sha256': digest(path), 'size': path.stat().st_size}


def finalize(repo, sources, installer, output, version):
    output.mkdir(parents=True, exist_ok=True)
    source_zip = output / f'OpenFO4-{version}-sources.zip'
    pair = output / f'OpenFO4-{version}-release-pair.json'
    if source_zip.exists() or pair.exists():
        raise ReleaseError('Release assets already exist; use a new output directory or version.')
    with zipfile.ZipFile(source_zip, 'x', compression=zipfile.ZIP_DEFLATED) as archive:
        for path in tree_files(sources):
            archive.write(path, path.relative_to(sources).as_posix())
    write(pair, {'schema': 1, 'version': version, 'commit': git(repo, 'rev-parse', 'HEAD'),
                 'installer': file_record(output, installer), 'sources': file_record(output, source_zip)})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('prepare', 'stage', 'finalize'))
    parser.add_argument('--repo', type=Path, required=True)
    parser.add_argument('--payload', type=Path)
    parser.add_argument('--sources', type=Path)
    parser.add_argument('--installer', type=Path)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--version')
    args = parser.parse_args()
    repo = args.repo.resolve()
    if args.command == 'prepare':
        if git(repo, 'status', '--porcelain', '--untracked-files=no'):
            raise ReleaseError('Build releases from a clean tracked checkout so the source archive matches the build.')
        prepare(repo)
    elif args.command == 'stage':
        stage(repo, args.payload, args.sources, args.version)
    else:
        finalize(repo, args.sources, args.installer, args.output, args.version)


if __name__ == '__main__':
    main()
