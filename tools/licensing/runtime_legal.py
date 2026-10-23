"""Copy the bundled Microsoft notices and their scoped component list."""
import argparse
import json
from pathlib import Path
import shutil


def prepare_release(repo, payload):
    ledger = json.loads((repo / 'tools/licensing/native-components.json').read_text(encoding='utf-8-sig'))
    owner = next(component for component in ledger['components'] if component['id'] == 'native:msvc-runtime')
    output = payload / 'legal'
    output.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(repo / 'tools/licensing/licenses/native/msvc-runtime/MICROSOFT_RUNTIME_TERMS.review.txt',
                    output / 'MICROSOFT_RUNTIME_TERMS.txt')
    component_list = {'schema': 1, 'scope': 'Only the listed Microsoft runtime files; other component licenses remain unchanged.',
                      'components': [{'component': owner['id'], 'licenseExpression': owner['licenseExpression'],
                                      'path': entry['path'], 'version': owner['version']}
                                     for entry in owner['runtimeFiles']]}
    (output / 'MICROSOFT_RUNTIME_COMPONENTS.json').write_text(json.dumps(component_list, indent=2) + '\n', encoding='utf-8')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, required=True)
    parser.add_argument('--payload', type=Path, required=True)
    args = parser.parse_args()
    prepare_release(args.repo, args.payload)
