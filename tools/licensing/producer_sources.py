"""Construct matching preferred-source archives for the bundled tools."""
import json
import subprocess
from pathlib import Path
import zipfile

from artifact_safety import digest, safe_name, tree_files


def prepare_ba2_source(repo):
    repo = Path(repo).resolve()
    selection = json.loads((repo / 'tools/licensing/ba2-source-inputs.json').read_text())
    baseline = selection['baseline']
    source = repo / safe_name(baseline['path'])
    if digest(source) != baseline['sha256']:
        raise ValueError('BA2Sharp upstream source baseline changed.')
    target = repo / 'build/cache/legal-sources/openfo4-ba2sharp-selected-source.zip'
    candidate = target.with_suffix('.candidate.zip')
    if candidate.exists():
        raise ValueError('Inspect the interrupted BA2Sharp source selection before retrying.')
    with zipfile.ZipFile(source) as upstream, zipfile.ZipFile(candidate, 'x') as prepared:
        seen = set()
        for record in selection['selectedMembers']:
            member, name = safe_name(record['member']), safe_name(record['path'])
            if name.casefold() in seen or name == selection['excludedRestrictedDdsPath']:
                raise ValueError('Duplicate or prohibited BA2Sharp source selection.')
            seen.add(name.casefold())
            content = upstream.read(member)
            import hashlib
            if hashlib.sha256(content).hexdigest() != record['sha256']:
                raise ValueError(f'BA2Sharp selected source member changed: {name}')
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            prepared.writestr(info, content)
        info = zipfile.ZipInfo('OPENFO4-SOURCE-SELECTION.json', (1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        prepared.writestr(info, json.dumps(selection, indent=2) + '\n')
    if target.exists():
        if digest(candidate) != digest(target):
            raise ValueError('BA2Sharp selected source archive changed.')
        candidate.unlink()
    else:
        candidate.rename(target)
    return target


def prepare_ffdec_source(repo):
    sources = repo / 'build/ffdec-sources'
    preferred = sources / 'preferred-form'
    if not preferred.is_dir() or not (sources / 'SOURCES.md').is_file():
        raise ValueError('Build FFDec with its matching preferred-form sources first.')
    target = repo / 'build/cache/legal-sources/openfo4-ffdec-package-source.zip'
    target.parent.mkdir(parents=True, exist_ok=True)
    tracked = subprocess.check_output(['git', '-C', str(repo), 'ls-files', '-z', '--', 'tools/ffdec']).decode().split('\0')
    instructions = (sources / 'SOURCES.md').read_text(encoding='utf-8-sig')
    instructions = instructions.replace(
        '`source-files.json` records every source/resource hash, compiler identity, exact acquisition source pins, original reference JAR hashes and modifications.',
        '`build-tools/toolchain.lock.json` records the exact acquisition source pins, dependencies and modifications.')
    instructions = instructions.replace(' (tested compiler is recorded in source-files.json)', '')
    with zipfile.ZipFile(target, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
        for path in tree_files(preferred):
            archive.write(path, path.relative_to(sources).as_posix())
        archive.writestr('SOURCES.md', instructions)
        for name in tracked:
            if not name:
                continue
            relative = Path(name).relative_to('tools/ffdec')
            root_tool = len(relative.parts) == 1 and relative.suffix in ('.ps1', '.json', '.md')
            support_file = relative.parts[0] == 'licenses' or (relative.parts[0] == 'tests' and relative.suffix == '.py')
            path = repo / name
            if (root_tool or support_file) and path.is_file():
                if path.is_symlink():
                    raise ValueError('FFDec build tools must be regular tracked files.')
                archive.write(path, 'build-tools/' + relative.as_posix())
    return target
