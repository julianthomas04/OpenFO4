"""Acquire only reviewed, hashed source inputs and exact license members."""

import hashlib
import subprocess
import tarfile
import tempfile
import urllib.request
import zipfile

from artifact_safety import digest, safe_name


def download(url, partial):
    with urllib.request.urlopen(url, timeout=60) as remote, partial.open('xb') as output:
        while block := remote.read(1024 * 1024):
            output.write(block)


def archive_git_commit(url, commit, prefix, partial):
    """Write the deterministic `git archive` tar of one pinned commit, the form its pinned hash was recorded from."""
    with tempfile.TemporaryDirectory() as repository:
        for arguments in (['init', '--quiet', '--bare', repository],
                          ['-C', repository, 'fetch', '--quiet', '--depth', '1', url, commit],
                          ['-C', repository, 'archive', '--format=tar', f'--prefix={prefix}/', f'--output={partial}', commit]):
            subprocess.run(['git', *arguments], check=True)


def acquire_sources(repo, components, resolve):
    allowed_root = (repo / 'build/cache').resolve()
    for component in components:
        if component.get('role') != 'runtime':
            continue
        inputs = [entry for source in component.get('sourceInputs', [])
                  for entry in [source, *source.get('additionalSources', [])]]
        for entry in inputs:
            if not entry.get('path') or not entry.get('sha256') or not entry.get('url'):
                continue
            target = resolve(repo, entry['path'])
            if target.is_file():
                if digest(target) != entry['sha256']:
                    raise ValueError(f'Cached source hash mismatch: {target}')
                continue
            if not target.resolve().is_relative_to(allowed_root):
                # Package-manager caches must be populated by their own restore operation.
                continue
            if not entry['url'].startswith('https://'):
                raise ValueError(f'Source acquisition requires HTTPS: {entry["url"]}')
            target.parent.mkdir(parents=True, exist_ok=True)
            partial = target.with_name(target.name + '.download')
            if partial.exists():
                raise ValueError(f'Remove or inspect interrupted source download: {partial}')
            if 'commit' in entry:
                archive_git_commit(entry['url'], entry['commit'], target.stem, partial)
            else:
                download(entry['url'], partial)
            if digest(partial) != entry['sha256']:
                raise ValueError(f'Downloaded source hash mismatch; preserved for inspection: {partial}')
            partial.rename(target)
        for entry in component.get('licenseFiles', []):
            if 'fromSource' not in entry:
                continue
            target = resolve(repo, entry['path'])
            if target.is_file() and digest(target) == entry['sha256']:
                continue
            if not target.resolve().is_relative_to(allowed_root):
                raise ValueError(f'License extraction outside build/cache refused: {target}')
            source = entry['fromSource']
            archive = resolve(repo, source['path'])
            member = safe_name(source['member'])
            if zipfile.is_zipfile(archive):
                with zipfile.ZipFile(archive) as container:
                    data = container.read(member)
            else:
                with tarfile.open(archive) as container:
                    info = container.getmember(member)
                    if not info.isfile():
                        raise ValueError(f'License member is not a regular file: {member}')
                    data = container.extractfile(info).read()
            if hashlib.sha256(data).hexdigest() != entry['sha256']:
                raise ValueError(f'License member hash mismatch: {member}')
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
