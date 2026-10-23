"""Walks the files that would be committed and applies every rule, minus what the allow-list waives."""

import os
import subprocess
from dataclasses import dataclass
from pathlib import Path

import byte_rules
import path_rules
import text_rules
from allowlist import AllowEntry
from finding import Finding

MAX_TEXT_BYTES = 1 << 20

RULE_IDS = frozenset({
    "forbidden-extension",
    "forbidden-directory",
    "game-magic",
    "embedded-dxbc",
    "unexpected-binary",
    "symlink",
    "unreadable",
    "oversized-text",
    "translated-wgsl",
    "dxbc-disassembly",
    "encoded-blob",
    "hex-byte-table",
    "numeric-table",
})


class GuardError(Exception):
    pass


@dataclass
class ScanResult:
    findings: list[Finding]
    waived: int
    unused_entries: list[AllowEntry]
    files_scanned: int

    @property
    def passed(self) -> bool:
        return not self.findings and not self.unused_entries


def list_git_files(root: Path) -> list[str]:
    """Tracked files plus untracked files that are not ignored: exactly what a commit could pick up."""
    try:
        completed = subprocess.run(
            ["git", "-C", str(root), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
            capture_output=True, check=True,
        )
    except FileNotFoundError as error:
        raise GuardError("git is not installed; use --no-git to scan a plain directory") from error
    except subprocess.CalledProcessError as error:
        raise GuardError(f"git ls-files failed in {root}: {error.stderr.decode(errors='replace').strip()}") from error
    paths = completed.stdout.decode("utf-8").split("\0")
    return sorted({path for path in paths if path and os.path.lexists(root / path)})


def list_directory_files(root: Path) -> list[str]:
    return sorted(
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if (path.is_file() or path.is_symlink()) and ".git" not in path.relative_to(root).parts
    )


def scan_file(root: Path, relative_path: str) -> list[Finding]:
    findings = path_rules.check_path(relative_path)
    full_path = root / relative_path
    if full_path.is_symlink():
        return findings + [Finding("symlink", relative_path, "tracked symbolic links can point at content outside the repository")]
    try:
        data = full_path.read_bytes()
    except OSError as error:
        return findings + [Finding("unreadable", relative_path, f"cannot read file: {error}")]

    findings += byte_rules.check_signatures(relative_path, data)
    findings += byte_rules.check_embedded_dxbc(relative_path, data)
    if byte_rules.is_binary(data):
        return findings + [Finding("unexpected-binary", relative_path, "binary files must be allow-listed with a justification")]
    if len(data) > MAX_TEXT_BYTES:
        findings.append(Finding("oversized-text", relative_path, f"text file of {len(data)} bytes exceeds {MAX_TEXT_BYTES}"))
    text = data.decode("utf-8", errors="replace").replace("\r\n", "\n")
    return findings + text_rules.check_translated_wgsl(relative_path, text) \
        + text_rules.check_dxbc_assembly(relative_path, text) \
        + text_rules.check_bulk_data(relative_path, text)


def scan(root: Path, relative_paths: list[str], allowlist: list[AllowEntry]) -> ScanResult:
    findings: list[Finding] = []
    waived = 0
    for relative_path in relative_paths:
        for finding in scan_file(root, relative_path):
            entry = next((entry for entry in allowlist if entry.covers(finding.rule, finding.path)), None)
            if entry is None:
                findings.append(finding)
            else:
                entry.waive(finding.rule)
                waived += 1
    unused = [entry for entry in allowlist if entry.unused_rules()]
    return ScanResult(findings, waived, unused, len(relative_paths))
