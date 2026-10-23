"""Allow-list of known-good exceptions. Every entry names its paths, the rules it waives and why that is safe."""

import re
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

MIN_JUSTIFICATION_CHARS = 20
ENTRY_KEYS = {"paths", "rules", "justification"}


class AllowlistError(Exception):
    pass


def glob_to_regex(pattern: str) -> re.Pattern[str]:
    """'*' and '?' stay inside one path segment, '**' crosses segments, '**/' also matches no directory at all."""
    regex = []
    position = 0
    while position < len(pattern):
        if pattern.startswith("**/", position):
            regex.append("(?:.*/)?")
            position += 3
        elif pattern.startswith("**", position):
            regex.append(".*")
            position += 2
        elif pattern[position] == "*":
            regex.append("[^/]*")
            position += 1
        elif pattern[position] == "?":
            regex.append("[^/]")
            position += 1
        else:
            regex.append(re.escape(pattern[position]))
            position += 1
    return re.compile("".join(regex) + r"\Z")


@dataclass
class AllowEntry:
    patterns: tuple[str, ...]
    rules: frozenset[str]
    justification: str
    regexes: tuple[re.Pattern[str], ...] = field(init=False)
    waived_rules: set[str] = field(default_factory=set, init=False)

    def __post_init__(self) -> None:
        self.regexes = tuple(glob_to_regex(pattern) for pattern in self.patterns)

    def covers(self, rule: str, path: str) -> bool:
        return rule in self.rules and any(regex.match(path) for regex in self.regexes)

    def waive(self, rule: str) -> None:
        self.waived_rules.add(rule)

    def unused_rules(self) -> frozenset[str]:
        return self.rules - self.waived_rules

    def describe(self) -> str:
        return f"paths {list(self.patterns)} rules {sorted(self.unused_rules())}"


def load_allowlist(path: Path, known_rules: frozenset[str]) -> list[AllowEntry]:
    try:
        with path.open("rb") as handle:
            document = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise AllowlistError(f"cannot read allow-list {path}: {error}") from error

    unknown_tables = set(document) - {"allow"}
    if unknown_tables:
        raise AllowlistError(f"{path}: unknown top-level keys {sorted(unknown_tables)}; entries are [[allow]] tables")
    return [_parse_entry(path, index, raw, known_rules) for index, raw in enumerate(document.get("allow", []), start=1)]


def _parse_entry(path: Path, index: int, raw: dict, known_rules: frozenset[str]) -> AllowEntry:
    where = f"{path} entry #{index}"
    if set(raw) != ENTRY_KEYS:
        raise AllowlistError(f"{where}: keys must be exactly {sorted(ENTRY_KEYS)}, got {sorted(raw)}")
    patterns, rules, justification = raw["paths"], raw["rules"], raw["justification"]
    if not isinstance(patterns, list) or not patterns or not all(isinstance(item, str) and item for item in patterns):
        raise AllowlistError(f"{where}: 'paths' must be a non-empty list of glob strings")
    if not isinstance(rules, list) or not rules or not all(isinstance(item, str) for item in rules):
        raise AllowlistError(f"{where}: 'rules' must be a non-empty list of rule ids")
    unknown_rules = set(rules) - known_rules
    if unknown_rules:
        raise AllowlistError(f"{where}: unknown rule ids {sorted(unknown_rules)}; known: {sorted(known_rules)}")
    if not isinstance(justification, str) or len(justification.strip()) < MIN_JUSTIFICATION_CHARS:
        raise AllowlistError(f"{where}: 'justification' must explain why the exception is safe (at least {MIN_JUSTIFICATION_CHARS} characters)")
    return AllowEntry(tuple(patterns), frozenset(rules), justification.strip())
