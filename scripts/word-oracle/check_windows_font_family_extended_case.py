"""Assert versioned family-case identity and retain pending absolute Word differences."""
import argparse
import json
from pathlib import Path

from check_platform_coordinates import ROOT, checked_file, compare, pdf_origins, read_json, require
from check_platform_coordinates import assert_pending_pdf_text
from check_windows_font_family_case import render, sha
from check_windows_font_family_cjk_case import verify_source_pair
from check_windows_font_coordinates import require_original_font_programs_registered
from check_windows_xps import assert_model
from extract_windows_emf import extract
from paragraph_backgrounds import compare_paragraph_backgrounds
from windows_xps_fonts import extract_xps_fonts

INDEX = Path('tests/word-oracle/windows/font-family-extended-cases.json')
INDEX_SHA256 = '608d1d8085fef44daa8ebb8b3951080a2cb4a4c6ad7ea470c1b30a7ef099cf64'
FAMILIES = {'Century': 'cEnTuRy', 'Bookman Old Style': 'bOoKmAn oLd sTyLe',
            'Yu Mincho': 'yU mInChO', 'MS Mincho': 'mS mInChO'}
STYLES = ('regular', 'bold', 'italic', 'bold-italic')


def required_word_coordinates(case):
    family = case['canonical_family']
    known = family in ('Yu Mincho', 'MS Mincho')
    return known and (case['compatibility_mode'] == 15 or case['line_ending'] == 'paragraph')


def verify(root=ROOT):
    require(sha(root / INDEX) == INDEX_SHA256, 'Extended family-case index changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and len(index['fixtures']) == 240,
            'Extended family-case coverage removed')
    names, controls, coverage, cases = set(), set(), set(), []
    for case in index['fixtures']:
        require(case['name'] not in names, 'Duplicate extended family-case source')
        names.add(case['name'])
        family = case['canonical_family']
        require(family in FAMILIES and case['variant'] in ('lower', 'upper', 'mixed'),
                'Extended family classification changed')
        spellings = dict(lower=family.lower(), upper=family.upper(), mixed=FAMILIES[family])
        require(case['family'] == spellings[case['variant']] and case['style'] in STYLES,
                'Extended family spelling/style changed')
        require(type(case['required_word_coordinates']) is bool
                and case['required_word_coordinates'] == required_word_coordinates(case),
                'Absolute Word requirement changed')
        groups = []
        for record in (case, case['canonical']):
            paths = {key: checked_file(root, record[key])
                     for key in ('source', 'capture', 'pdf', 'xps')}
            capture = read_json(paths['capture'])
            require(capture['word'] == index['word'] and capture['status'] == 'complete'
                    and capture['platform'] == 'windows', 'Extended Word identity changed')
            document, = [d for d in capture['documents'] if d['name'] == record['name']]
            require(document['sha256'] == record['source']['sha256']
                    and document['open_mode'] == 'normal-read-only'
                    and document['compatibility_mode'] == case['compatibility_mode'],
                    'Extended source/open-mode/compatibility changed')
            for key in ('pdf', 'xps'):
                require(paths[key] == paths['capture'].parent / document[key]
                        and record[key]['sha256'] == document[key + '_sha256'],
                        'Extended original Word output changed')
            groups.append(paths)
        alias, canonical = groups
        verify_source_pair(alias['source'], canonical['source'], case['family'], family)
        require(pdf_origins(alias['pdf']) == pdf_origins(canonical['pdf']),
                'Original Word coordinates differ across extended family spellings')
        controls.add(case['canonical']['source']['sha256'])
        coverage.add((family, case['style'], case['variant'], case['compatibility_mode'],
                      case['line_ending'], case['line_width_twips']))
        cases.append((case, alias, canonical))
    expected = {(f, s, v, m, e, w) for f in FAMILIES
                for s in (STYLES if f in ('Yu Mincho', 'MS Mincho') else ('regular',))
                for v in ('lower', 'upper', 'mixed') for m in (14, 15)
                for e in ('manual', 'paragraph') for w in (3000, 8504)}
    require(len(controls) == 80 and coverage == expected
            and sum(c['required_word_coordinates'] for c, _, _ in cases) == 144,
            'Extended family/style/Word-assertion coverage removed')
    return cases


def observe_word(record, paths, native, painted, emf):
    name = record['name']
    # Pending positions never excuse missing/reordered text, pages or nonfinite output.
    for unclipped in (False, True):
        expected = pdf_origins(paths['pdf'], unclipped)
        assert_pending_pdf_text(expected, native, name + '/native')
        assert_pending_pdf_text(expected, pdf_origins(painted, unclipped), name + '/painted')
    planes = {}
    for label, expected, actual in [
        ('EMF', emf, native),
        ('PDF-unclipped', pdf_origins(paths['pdf'], True), native),
        ('PDF-visible', pdf_origins(paths['pdf']), native),
        ('painted-PDF', pdf_origins(paths['pdf']), pdf_origins(painted)),
    ]:
        try:
            compare(expected, actual, name + '/' + label)
            planes[label] = 'PASS'
        except ValueError as error:
            planes[label] = str(error)
    for label in ('XPS-native', 'XPS-visible'):
        try:
            assert_model(pdf_origins(paths['xps'], True), native, name + '/' + label, True)
            planes[label] = 'PASS'
        except ValueError as error:
            planes[label] = str(error)
    try:
        compare_paragraph_backgrounds(paths['source'], paths['pdf'], painted, name)
        background = 'PASS'
    except ValueError as error:
        background = str(error)
    return dict(planes=planes, paragraph_background=background)


def require_promoted_word(case, observed):
    if case['required_word_coordinates']:
        require(all(value == 'PASS' for value in observed['planes'].values())
                and observed['paragraph_background'] == 'PASS',
                case['name'] + ': promoted original Word coordinate regression: ' + str(observed))


def check(executable, output, root=ROOT):
    cases = verify(root)
    executable = executable.resolve()
    native_sha = sha(executable)
    output.mkdir(parents=True, exist_ok=False)
    controls, emfs, rows = {}, {}, []

    def observation(record, paths, native, target):
        if paths['capture'] not in emfs:
            emfs[paths['capture']] = {r['name']: r for r in extract(paths['capture'])['fixtures']}
        return observe_word(record, paths, native, target / 'native.pdf',
                            emfs[paths['capture']][record['name']])

    for number, (case, alias, canonical) in enumerate(cases):
        folder = output / f'c{number:03d}'
        folder.mkdir()
        fonts = folder / 'fonts'
        extract_xps_fonts(alias['xps'], fonts, output / 'font-cache')
        programs = tuple(sorted(sha(p) for p in fonts.iterdir()
                                if p.suffix.lower() in ('.ttf', '.otf', '.ttc')))
        require(bool(programs), case['name'] + ': original fonts absent')
        key = case['canonical']['source']['sha256'], programs
        if key not in controls:
            target = output / f't{len(controls):03d}'
            control = render(executable, canonical['source'], target, fonts, root)
            require(control['input_sha256'] == key[0], 'Extended canonical source changed')
            require_original_font_programs_registered(fonts, control)
            observed = observation(case['canonical'], canonical, control, target)
            require_promoted_word(dict(case, name=case['canonical']['name']), observed)
            controls[key] = control, target, observed
        control, target, control_word = controls[key]
        actual = render(executable, alias['source'], folder / 'alias', fonts, root)
        require(actual['input_sha256'] == case['source']['sha256'], 'Extended alias source changed')
        require_original_font_programs_registered(fonts, actual)
        require(actual['page_count'] == control['page_count'] and actual['glyphs'] == control['glyphs'],
                case['name'] + ': extended family-case origins differ from canonical family')
        require((folder / 'alias/native.png').read_bytes() == (target / 'native.png').read_bytes(),
                case['name'] + ': extended family case changed screen paint')
        require(pdf_origins(folder / 'alias/native.pdf') == pdf_origins(target / 'native.pdf'),
                case['name'] + ': extended family case changed painted origins')
        observed = observation(case, alias, actual, folder / 'alias')
        require_promoted_word(case, observed)
        rows.append(dict(name=case['name'], source_sha256=case['source']['sha256'],
                         native_case_glyphs_screen_PDF_exact=True,
                         required_word_coordinates=case['required_word_coordinates'],
                         original_Word=observed, canonical_original_Word=control_word,
                         strict_text_page_order_finite='PASS'))
        print(case['name'] + ': case identity PASS; original Word '
              + ('required PASS' if case['required_word_coordinates'] else 'pending observation retained'),
              flush=True)
    require(len(rows) == 240 and sha(executable) == native_sha, 'Extended case/native coverage changed')
    (output / 'results.json').write_bytes((json.dumps(dict(native_sha256=native_sha, cases=rows),
                                                     indent=2) + '\n').encode())
    return rows


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output', type=Path, default=Path('target/windows-font-family-extended-cases'))
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable, args.output)
    else:
        print(f'Verified {len(verify())} original Word extended family-case controls')
