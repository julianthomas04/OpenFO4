"""Shared filesystem and archive-path helpers for source packaging."""

import hashlib
import os
import re
import stat
from pathlib import Path, PurePosixPath


class UnsafeArtifact(ValueError):
    pass


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def safe_name(name):
    name = name.replace('\\', '/')
    parts = PurePosixPath(name).parts
    if not name or name.startswith('/') or re.match(r'^[A-Za-z]:', name) or '..' in parts or '\x00' in name:
        raise UnsafeArtifact(f'Unsafe archive path: {name!r}')
    if any(':' in part or part.endswith((' ', '.')) for part in parts):
        raise UnsafeArtifact(f'Ambiguous Windows archive path: {name!r}')
    raw_parts = name.rstrip('/').split('/')
    if any(part in ('', '.') for part in raw_parts):
        raise UnsafeArtifact(f'Noncanonical archive path: {name!r}')
    reserved = {'con', 'prn', 'aux', 'nul', 'conin$', 'conout$',
                *(f'com{number}' for number in range(1, 10)), *(f'lpt{number}' for number in range(1, 10))}
    if any(part.split('.', 1)[0].casefold() in reserved for part in raw_parts):
        raise UnsafeArtifact(f'Reserved Windows archive path: {name!r}')
    return name


def tree_files(root):
    root = Path(root).absolute()

    def visit(directory):
        metadata = directory.lstat()
        if stat.S_ISLNK(metadata.st_mode) or getattr(metadata, 'st_file_attributes', 0) & 0x400:
            raise UnsafeArtifact(f'Linked release entry: {directory}')
        with os.scandir(directory) as entries:
            paths = sorted(Path(entry.path) for entry in entries)
        for path in paths:
            metadata = path.lstat()
            if stat.S_ISLNK(metadata.st_mode) or getattr(metadata, 'st_file_attributes', 0) & 0x400:
                raise UnsafeArtifact(f'Linked release entry: {path}')
            if stat.S_ISDIR(metadata.st_mode):
                yield from visit(path)
            elif stat.S_ISREG(metadata.st_mode):
                yield path
            else:
                raise UnsafeArtifact(f'Nonregular release entry: {path}')

    yield from visit(root)
