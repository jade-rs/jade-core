"""Assert original Windows Word family-case coordinates for four Latin faces."""
import argparse
from pathlib import Path

from check_platform_coordinates import ROOT, checked_file, pdf_origins, read_json, require
from check_windows_font_family_case import sha
from check_windows_font_family_cjk_case import check_cases, verify_source_pair

INDEX = Path('tests/word-oracle/windows/font-family-other-cases.json')
INDEX_SHA256 = '8bd3d1351b1694c97ddacd77ad54ee512ef6064c0c8eb3b68581666b1ce81702'
FAMILIES = {
    'Arial': 'aRiAl', 'Calibri': 'cAlIbRi',
    'Times New Roman': 'tImEs nEw rOmAn', 'Aptos': 'aPtOs',
}


def verify(root=ROOT):
    require(sha(root / INDEX) == INDEX_SHA256, 'Other-family case index changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and len(index['fixtures']) == 96,
            'Other-family case coverage removed')
    names, controls, coverage, cases = set(), set(), set(), []
    for case in index['fixtures']:
        require(case['name'] not in names, 'Duplicate other-family case')
        names.add(case['name'])
        family = case['canonical_family']
        require(family in FAMILIES and case['variant'] in ('lower', 'upper', 'mixed'),
                'Other-family classification changed')
        spellings = dict(lower=family.lower(), upper=family.upper(), mixed=FAMILIES[family])
        require(case['family'] == spellings[case['variant']], 'Other-family spelling changed')
        groups = []
        for record in (case, case['canonical']):
            paths = {key: checked_file(root, record[key])
                     for key in ('source', 'capture', 'pdf', 'xps')}
            capture = read_json(paths['capture'])
            require(capture['word'] == index['word'], 'Other-family Word identity changed')
            document, = [d for d in capture['documents'] if d['name'] == record['name']]
            require(document['sha256'] == record['source']['sha256']
                    and document['open_mode'] == 'normal-read-only',
                    'Other-family source/open mode changed')
            for key in ('pdf', 'xps'):
                require(paths[key] == paths['capture'].parent / document[key]
                        and record[key]['sha256'] == document[key + '_sha256'],
                        'Other-family original output changed')
            groups.append(paths)
        alias, canonical = groups
        verify_source_pair(alias['source'], canonical['source'], case['family'], family)
        require(pdf_origins(alias['pdf']) == pdf_origins(canonical['pdf']),
                'Original Word coordinates differ across other-family spellings')
        controls.add(case['canonical']['source']['sha256'])
        coverage.add((family, case['variant'], case['compatibility_mode'],
                      case['line_ending'], case['line_width_twips']))
        cases.append((case, alias, canonical))
    require(len(controls) == 32 and coverage == {
        (f, v, m, e, w) for f in FAMILIES for v in ('lower', 'upper', 'mixed')
        for m in (14, 15) for e in ('manual', 'paragraph') for w in (3000, 8504)
    }, 'Other-family/mode/ending/width coverage removed')
    return cases


def check(executable, output, root=ROOT):
    return check_cases(executable, output, verify(root), root)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output', type=Path, default=Path('target/windows-font-family-other-cases'))
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable, args.output)
    else:
        print(f'Verified {len(verify())} original Word other-family case controls')
