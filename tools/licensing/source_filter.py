"""Prepare pinned preferred sources while excluding individually reviewed fixtures."""

import hashlib
import json
from pathlib import Path, PurePosixPath
import posixpath
import stat
import tarfile
import zipfile
import xml.etree.ElementTree as ET

from artifact_safety import digest, safe_name


def filter_archive(source, target, entry, resolve_additional=None):
    """Create a deterministic ZIP retaining regular files and exact reviewed aliases."""
    if digest(source) != entry['sha256']:
        raise ValueError('Upstream source bytes changed.')
    exclusions = {item['path']: item for item in entry.get('excludedFiles', [])}
    links = {item['path']: item for item in entry.get('materializedLinks', [])}
    if not exclusions and not links:
        raise ValueError('Explicit source exclusions or link resolutions are required.')
    supplements = {}
    for additional in entry.get('additionalSources', []):
        if resolve_additional is None:
            raise ValueError('Supplemental source resolver is required.')
        archive_path = resolve_additional(additional['path'])
        if digest(archive_path) != additional['sha256']:
            raise ValueError('Supplemental source bytes changed.')
        prefix = safe_name(additional['stripPrefix']).rstrip('/') + '/'
        destination = safe_name(additional['destination']).rstrip('/') + '/'
        with zipfile.ZipFile(archive_path) as archive:
            for info in archive.infolist():
                name = safe_name(info.filename.rstrip('/'))
                if info.is_dir():
                    continue
                if not name.startswith(prefix) or stat.S_ISLNK(info.external_attr >> 16):
                    raise ValueError('Unsafe supplemental source entry.')
                mapped = safe_name(destination + name[len(prefix):])
                if mapped.casefold() in {value.casefold() for value in supplements}:
                    raise ValueError('Duplicate supplemental source entry.')
                supplements[mapped] = (archive.read(info), info.external_attr)
    seen, removed, materialized = set(), set(), set()
    is_zip = zipfile.is_zipfile(source)
    container = zipfile.ZipFile(source) if is_zip else tarfile.open(source, 'r:*')
    with container, zipfile.ZipFile(target, 'x', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as output:
        entries = container.infolist() if is_zip else container.getmembers()
        by_name = {}
        for info in entries:
            raw = info.filename if is_zip else info.name
            name = safe_name(raw.rstrip('/'))
            if name.casefold() in seen:
                raise ValueError(f'Duplicate source entry: {name}')
            seen.add(name.casefold())
            by_name[name] = info

        def regular(info):
            return not stat.S_ISLNK(info.external_attr >> 16) and not info.is_dir() if is_zip else info.isfile()

        def read(info):
            if not regular(info):
                raise ValueError('Link target must be an upstream regular file.')
            return container.read(info) if is_zip else container.extractfile(info).read()

        for name, info in sorted(by_name.items()):
            if info.is_dir() if is_zip else info.isdir():
                continue
            linked = stat.S_ISLNK(info.external_attr >> 16) if is_zip else info.issym()
            if linked:
                resolution = links.get(name)
                raw_target = container.read(info).decode('utf-8') if is_zip else info.linkname
                if not resolution or raw_target != resolution['target']:
                    raise ValueError(f'Unreviewed source link: {name}')
                resolved = posixpath.normpath(posixpath.join(posixpath.dirname(name), raw_target))
                safe_name(resolved)
                if (resolved != resolution['resolvedTargetMember'] or resolved in exclusions
                        or (resolved not in by_name and resolved not in supplements)):
                    raise ValueError(f'Unsafe or excluded source link target: {name}')
                data = read(by_name[resolved]) if resolved in by_name else supplements[resolved][0]
                if hashlib.sha256(data).hexdigest() != resolution['targetSha256']:
                    raise ValueError(f'Source link target bytes changed: {name}')
                materialized.add(name)
            else:
                data = read(info)
            if name in exclusions:
                expected = exclusions[name]
                if not expected.get('reason') or hashlib.sha256(data).hexdigest() != expected['sha256']:
                    raise ValueError(f'Fixture exclusion no longer matches reviewed bytes: {name}')
                removed.add(name)
                continue
            prepared = zipfile.ZipInfo(name)
            prepared.compress_type = zipfile.ZIP_DEFLATED
            mode = (info.external_attr >> 16) if is_zip else info.mode
            prepared.external_attr = (stat.S_IFREG | (mode & 0o777)) << 16
            output.writestr(prepared, data)
        for name, (data, mode) in sorted(supplements.items()):
            if name.casefold() in seen:
                raise ValueError(f'Supplemental source collision: {name}')
            prepared = zipfile.ZipInfo(name)
            prepared.compress_type = zipfile.ZIP_DEFLATED
            prepared.external_attr = (stat.S_IFREG | ((mode >> 16) & 0o777)) << 16
            output.writestr(prepared, data)
        if removed != exclusions.keys() or materialized != links.keys():
            raise ValueError('Reviewed source exclusions or aliases are absent upstream.')
        receipt = {'schema': 1, 'upstreamSha256': entry['sha256'],
                   'excludedFiles': list(exclusions.values()), 'materializedLinks': list(links.values()),
                   **({'additionalSources': entry['additionalSources']} if entry.get('additionalSources') else {})}
        output.writestr(zipfile.ZipInfo('OpenFO4-SOURCE-EXCLUSIONS.json'), json.dumps(receipt, indent=2) + '\n')
    if entry.get('preparedSha256') and digest(target) != entry['preparedSha256']:
        raise ValueError('Prepared source bytes changed.')


def filter_zip(source, target, entry, resolve_additional=None):
    if digest(source) != entry['sha256']:
        raise ValueError(f'Upstream source bytes changed: {source}')
    exclusions = {item['path']: item for item in entry.get('excludedFiles', [])}
    if not exclusions:
        raise ValueError('A filtered source package needs reviewed, exact exclusions.')
    removed = set()
    with zipfile.ZipFile(source) as upstream, zipfile.ZipFile(target, 'x', compression=zipfile.ZIP_DEFLATED) as prepared:
        seen = set()
        for info in upstream.infolist():
            name = safe_name(info.filename)
            if name.casefold() in seen:
                raise ValueError(f'Duplicate source entry: {name}')
            seen.add(name.casefold())
            if stat.S_ISLNK(info.external_attr >> 16):
                raise ValueError(f'Linked source entry: {name}')
            if info.is_dir():
                continue
            data = upstream.read(info)
            if name in exclusions:
                expected = exclusions[name]
                if not expected.get('reason') or hashlib.sha256(data).hexdigest() != expected['sha256']:
                    raise ValueError(f'Fixture exclusion no longer matches reviewed bytes: {name}')
                removed.add(name)
                continue
            info.compress_type = zipfile.ZIP_STORED
            prepared.writestr(info, data)
        if removed != exclusions.keys():
            raise ValueError('Some reviewed fixture exclusions no longer exist upstream.')
        for supplement in entry.get('additionalSources', []):
            if resolve_additional is None:
                raise ValueError('Supplemental source resolver is required.')
            supplemental_archive = resolve_additional(supplement['path'])
            if digest(supplemental_archive) != supplement['sha256']:
                raise ValueError('Supplemental source bytes changed.')
            prefix = safe_name(supplement['stripPrefix']).rstrip('/') + '/'
            destination = safe_name(supplement['destination']).rstrip('/') + '/'
            with zipfile.ZipFile(supplemental_archive) as additional:
                for info in additional.infolist():
                    name = safe_name(info.filename)
                    if info.is_dir():
                        continue
                    if not name.startswith(prefix) or stat.S_ISLNK(info.external_attr >> 16):
                        raise ValueError(f'Unsafe supplemental source entry: {name}')
                    mapped = destination + name[len(prefix):]
                    safe_name(mapped)
                    if mapped.casefold() in seen:
                        raise ValueError(f'Supplemental source collision: {mapped}')
                    seen.add(mapped.casefold())
                    data = additional.read(info)
                    output_info = zipfile.ZipInfo(mapped)
                    output_info.external_attr = info.external_attr
                    prepared.writestr(output_info, data)
        receipt = {'schema': 1, 'origin': entry['url'], 'upstreamSha256': entry['sha256'],
                   'excludedFiles': list(exclusions.values()), 'additionalSources': entry.get('additionalSources', []),
                   'reason': 'Unneeded test fixture data is excluded. Production library source, build files, license texts, and test source code remain.'}
        receipt_info = zipfile.ZipInfo('OpenFO4-SOURCE-EXCLUSIONS.json')
        prepared.writestr(receipt_info, json.dumps(receipt, indent=2) + '\n')
    expected = entry.get('preparedSha256')
    if expected and digest(target) != expected:
        raise ValueError(f'Prepared source bytes changed: {target}')


def project_closure(archive_path, root_project):
    """Prove retained production ProjectReference paths exist without executing upstream targets."""
    with zipfile.ZipFile(archive_path) as archive:
        names = set(archive.namelist())
        pending, visited = [root_project], set()
        while pending:
            project = safe_name(pending.pop())
            if project in visited:
                continue
            if project not in names:
                raise ValueError(f'Production source project is missing: {project}')
            if '.Test' in project or '.UnitTest' in project:
                raise ValueError(f'Production build unexpectedly depends on a test project: {project}')
            visited.add(project)
            root = ET.fromstring(archive.read(project))
            for item in root.iter():
                kind = item.tag.rsplit('}', 1)[-1]
                if kind not in ('ProjectReference', 'AdditionalFiles') or 'Include' not in item.attrib:
                    continue
                include = item.attrib['Include'].replace('\\', '/')
                if '$(' in include or '*' in include or ';' in include:
                    raise ValueError(f'Unresolved production source input in {project}: {include}')
                resolved = posixpath.normpath(str(PurePosixPath(project).parent / include))
                safe_name(resolved)
                if resolved not in names:
                    raise ValueError(f'Missing production source input: {resolved}')
                if kind == 'ProjectReference':
                    pending.append(resolved)
        return sorted(visited)
