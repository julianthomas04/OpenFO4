"""Apply exact source fixture exclusions and materialized build-source links."""
import json
from pathlib import Path


def enrich(repo, components):
    policy = json.loads((repo / 'tools/licensing/source-curation.json').read_text())
    for component in components:
        for entry in component.get('sourceInputs', []):
            checksum = entry.get('sha256')
            exclusions = [dict(path=row['member'], sha256=row['sha256'], reason=row['reason'])
                          for row in policy['sourceFilters'] if row['archiveSha256'] == checksum]
            links = [row for row in policy['materializedLinks'] if row['archiveSha256'] == checksum]
            if exclusions or links:
                entry['excludedFiles'] = entry.get('excludedFiles', []) + exclusions
                entry['materializedLinks'] = links
                supplements = [row['additionalSource'] for row in links if row.get('additionalSource')]
                if supplements:
                    entry['additionalSources'] = supplements
    return components
