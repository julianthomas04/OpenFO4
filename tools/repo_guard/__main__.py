"""Fails when Bethesda-derived material enters the tree. Exceptions are listed in tools/repo_guard/allowlist.toml.

Usage: python tools/repo_guard [--root DIR] [--allowlist FILE] [--no-git]
Exit status: 0 clean, 1 findings or stale allow-list entries, 2 the guard itself could not run.
"""

import argparse
import sys
from collections import defaultdict
from pathlib import Path

from allowlist import AllowlistError, load_allowlist
from scanner import RULE_IDS, GuardError, ScanResult, list_directory_files, list_git_files, scan

GUARD_DIRECTORY = Path(__file__).resolve().parent
DEFAULT_ROOT = GUARD_DIRECTORY.parents[1]
DEFAULT_ALLOWLIST = GUARD_DIRECTORY / "allowlist.toml"


def parse_arguments(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(prog="repo_guard", description="Fails when Bethesda-derived material enters the tree.")
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT, help="repository root (default: this checkout)")
    parser.add_argument("--allowlist", type=Path, default=DEFAULT_ALLOWLIST, help="allow-list file (default: tools/repo_guard/allowlist.toml)")
    parser.add_argument("--no-git", action="store_true", help="scan every file under --root instead of asking git which files would be committed")
    return parser.parse_args(argv)


def report(result: ScanResult) -> None:
    by_rule = defaultdict(list)
    for finding in result.findings:
        by_rule[finding.rule].append(finding)
    for rule in sorted(by_rule):
        print(f"\n[{rule}]")
        for finding in sorted(by_rule[rule], key=lambda item: (item.path, item.line or 0)):
            print(f"  {finding.location()}: {finding.detail}")
    for entry in result.unused_entries:
        print(f"\n[stale-allowlist-entry]\n  {entry.describe()} never fires; remove it so it cannot hide a future file.\n  justification was: {entry.justification}")

    print(f"\nrepo_guard: scanned {result.files_scanned} files, {len(result.findings)} findings, {result.waived} waived by the allow-list.")
    if result.findings:
        print("Remove the flagged material. Game assets and anything derived from them must never be committed.")
        print("Only if a finding is a verified false positive, add a narrowly scoped, justified entry to tools/repo_guard/allowlist.toml.")


def main(argv: list[str]) -> int:
    arguments = parse_arguments(argv)
    root = arguments.root.resolve()
    try:
        allowlist = load_allowlist(arguments.allowlist, RULE_IDS)
        paths = list_directory_files(root) if arguments.no_git else list_git_files(root)
    except (AllowlistError, GuardError) as error:
        print(f"repo_guard: {error}", file=sys.stderr)
        return 2
    result = scan(root, paths, allowlist)
    report(result)
    return 0 if result.passed else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
