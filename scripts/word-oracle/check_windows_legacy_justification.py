"""Preserve original Word width/font controls and required/pending render gates."""
import argparse
import hashlib
from io import BytesIO
import json
import math
import subprocess
import sys
from pathlib import Path
from zipfile import ZipFile

from fontTools.ttLib import TTFont
import pymupdf

from check_platform_coordinates import ROOT, checked_file, pdf_origins, read_json, require
from check_windows_xps import assert_model
from extract_windows_emf import extract
import generate_windows_legacy_justification_probes as producer

INDEX = Path('tests/word-oracle/windows/legacy-justification-observations.json')
INDEX_SHA256 = '04be757ec488113a4193908bda44599553428dba134b96ec6724d70b104d5eb9'
FONT_NAMES_SHA256 = '326568b2eb0545899f09599dd9be310eeccb9065c55dd07236312ab44cafcfc8'
XPS_NAMES_SHA256 = '99fc9fd4aa274db1f1ef91551620875b4bb8ed7980f9424c40ede55183f3e99b'


def locked_promotions(index):
    rows = index['fixtures']
    require(index['schema'] == 1 and len(rows) == len({r['name'] for r in rows}) == 80,
            'Legacy justification source coverage changed')
    names = []
    for field, promotion, count, digest in (
            ('font_gate_promotion', 'legacy-justify-font', 68, FONT_NAMES_SHA256),
            ('xps_gate_promotion', 'legacy-justify-xps', 66, XPS_NAMES_SHA256)):
        selected = sorted(r['name'] for r in rows if r[field] == promotion)
        require(len(selected) == count and hashlib.sha256(
            json.dumps(selected, separators=(',', ':')).encode()).hexdigest() == digest,
            'Required legacy justification coordinate assertions changed')
        require(all(r[field] in (promotion, 'pending') for r in rows), 'Unknown legacy justification promotion')
        names.append(frozenset(selected))
    require(names[1] <= names[0], 'Required XPS output missing a font render')
    return tuple(names)


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Original legacy justification registry changed')
    index = read_json(root / INDEX)
    required, xps_required = locked_promotions(index)
    generated = {n: (data, spec) for n, data, spec in producer.generate(root)}
    require(set(generated) == {r['name'] for r in index['fixtures']}, 'Legacy justification input coverage changed')
    require(index['word']['product_version'] == '16.0.19127.20936'
            and index['word']['executable_sha256'] ==
            '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312',
            'Original legacy justification Word version/executable changed')
    captures, documents, drawings, xps_models = {}, {}, {}, {}
    require(len(index['captures']) == 2, 'Legacy justification capture coverage changed')
    extras = {'input-facts.json', 'generate-inputs.py', 'capture-wrapper.ps1',
              'capture-word-windows.ps1', 'word-windows-page-geometry.ps1',
              'word-output-font-facts.json', 'capture-provenance.json'}
    for record in index['captures']:
        manifest = checked_file(root, record['capture'])
        capture = read_json(manifest)
        require(capture['schema'] == 2 and capture['status'] == 'complete'
                and capture['platform'] == 'windows' and capture['word'] == index['word']
                and capture['capture_id'] == record['capture_id'], 'Original legacy justification capture identity changed')
        count = len(capture['documents'])
        require(count == record['documents'] and count in (32, 48), 'Legacy justification document coverage changed')
        with ZipFile(checked_file(root, record['archive'])) as archive, \
                ZipFile(checked_file(root, record['original_capture'])) as original:
            members = set(archive.namelist())
            require(len(archive.namelist()) == len(members) == 4 * count + 1
                    and len(original.namelist()) == len(set(original.namelist())) == 4 * count + 8
                    and set(original.namelist()) == members | extras,
                    'Original legacy justification archive coverage changed')
            require(archive.read('capture.json') == manifest.read_bytes(), 'Original capture metadata changed')
            for member in archive.namelist():
                require(archive.read(member) == original.read(member), 'Original capture output substituted')
            facts_file = checked_file(root, record['font_facts'])
            require(original.read('word-output-font-facts.json') == facts_file.read_bytes(), 'Original font facts changed')
            facts = {f['name']: f for f in read_json(facts_file)}
            require(len(facts) == count, 'Original font coverage changed')
            provenance = json.loads(original.read('capture-provenance.json'))
            require(provenance['word'] == capture['word'] and provenance['capture_id'] == capture['capture_id']
                    and provenance['capture_sha256'] == record['capture']['sha256']
                    and provenance['font_facts_sha256'] == record['font_facts']['sha256'],
                    'Original capture tool/font provenance changed')
            for member, digest in provenance['tools_sha256'].items():
                require(hashlib.sha256(original.read(member)).hexdigest() == digest, 'Original capture tool changed')
            for document in capture['documents']:
                name = document['name']
                require(name not in documents and name in generated, 'Duplicate/unknown captured input')
                data, specification = generated[name]
                require(document['sha256'] == hashlib.sha256(data).hexdigest()
                        and data == archive.read('inputs/' + name + '.docx')
                        and document['open_mode'] == 'normal-read-only'
                        and document['compatibility_mode'] == specification['compatibility_mode'],
                        'Original legacy justification source/open mode changed')
                for key in ('pdf', 'xps'):
                    path = manifest.parent / document[key]
                    require(path.read_bytes() == archive.read(document[key])
                            and hashlib.sha256(path.read_bytes()).hexdigest() == document[key + '_sha256'],
                            'Original legacy justification PDF/XPS changed')
                font_facts = facts[name]
                require(font_facts['word'] == capture['word'] and font_facts['source_sha256'] == document['sha256'],
                        'Original legacy justification font identity changed')
                with pymupdf.open(manifest.parent / document['pdf']) as pdf:
                    for font in font_facts['pdf_fonts']:
                        program = pdf.extract_font(font['xref'])[3]
                        require(hashlib.sha256(program).hexdigest() == font['program_sha256'], 'Original PDF font changed')
                        with TTFont(BytesIO(program)) as face:
                            require(face['name'].getDebugName(5) == font['version']
                                    and face['name'].getDebugName(1) == font['family'], 'Recorded font version changed')
                with ZipFile(manifest.parent / document['xps']) as package:
                    for font in font_facts['xps_font_members']:
                        require(hashlib.sha256(package.read(font['file'])).hexdigest() == font['sha256'],
                                'Original XPS font changed')
                documents[name] = (record, document, manifest)
        drawing = extract(manifest)
        require(read_json(checked_file(root, record['drawing_reference'])) == drawing,
                'Drawing reference differs from original Word EMF')
        drawings.update({d['name']: d for d in drawing['fixtures']})
        captures[record['capture_id']] = record
    require(len(documents) == 80, 'Legacy justification originals removed')
    fonts = {c['name']: c for c in read_json(root / 'tests/word-oracle/windows/font-coordinates.json')['fixtures']}
    for case in index['fixtures']:
        name = case['name']
        record, document, manifest = documents[name]
        data, specification = generated[name]
        require(checked_file(root, case['source']).read_bytes() == data
                and case['input_specification'] == specification
                and case['capture'] == record['capture'] and case['archive'] == record['archive']
                and case['capture_id'] == record['capture_id'] and case['word'] == index['word'],
                'Legacy justification original specification/reference changed')
        wanted = specification['expected_non_whitespace_glyphs']
        drawing = drawings[name]
        xps = pdf_origins(manifest.parent / document['xps'], True)
        require(drawing['page_count'] == xps['page_count'] == document['pages'] == 1
                and len(drawing['glyphs']) == case['glyph_count']
                and sum(not g[1].isspace() for g in drawing['glyphs']) == len(xps['glyphs']) == wanted,
                'Legacy justification page/glyph coverage changed')
        xps_models[name] = xps
        if name in required:
            require(name in fonts and fonts[name] == dict(name=name, capture=case['capture'],
                drawing_reference=record['drawing_reference'], source=case['source'],
                word_product_version=index['word']['product_version'], glyph_count=case['glyph_count'],
                public_test_password=None, solid_paragraph_background=True),
                'Required legacy justification font assertions removed/substituted')
        else:
            require(name not in fonts, 'Pending legacy legacy justification coordinates promoted without proof')
    require(sum(r.get('pending_font_reason') == 'word-pdf-subset-missing-kerning' for r in index['fixtures']) == 8
            and sum(r.get('pending_font_reason') == 'windows-modern-mixed-justification' for r in index['fixtures']) == 4,
            'Pending font-input/modern-justification distinction changed')
    return index['fixtures'], required, xps_required, xps_models, documents


def render_pending(executable, directory, root, cases, required, documents):
    """Keep text, pages, font programs and paint checked for unpromoted inputs."""
    from check_platform_coordinates import extract_fonts
    from check_windows_font_coordinates import require_original_font_programs_registered
    require(sys.platform == 'win32', 'Pending Windows controls require Windows')
    pending = [case for case in cases if case['name'] not in required]
    require(len(pending) == 12, 'Pending legacy source coverage changed')
    for case in pending:
        name = case['name']
        _, document, manifest = documents[name]
        source = checked_file(root, case['source'])
        target = directory / case['capture_id'] / name
        target.mkdir(parents=True, exist_ok=False)
        fonts = target / 'fonts'
        extract_fonts(manifest.parent / document['pdf'], fonts)
        for plane, extra in [('native', [str(target / 'native.pdf'), '--unclipped-tables']),
                             ('visible', [])]:
            destination = target / (plane + '.json')
            subprocess.run([str(executable), str(source), str(destination), 'windows',
                            *extra, '--fonts-directory', str(fonts)], check=True)
            require_original_font_programs_registered(fonts, read_json(destination))


def check_rendered_fonts(directory, root=ROOT, executable=None):
    from check_platform_coordinates import assert_pending_pdf_text
    from paragraph_backgrounds import compare_paragraph_backgrounds
    cases, font_required, xps_required, models, documents = verify(root)
    if executable is not None:
        render_pending(executable, directory, root, cases, font_required, documents)
    results = []
    for case in cases:
        name = case['name']
        target = directory / case['capture_id'] / name
        _, document, manifest = documents[name]
        original, painted = manifest.parent / document['pdf'], target / 'native.pdf'
        require(painted.read_bytes() != original.read_bytes(), 'Word self-comparison')
        for unclipped in (False, True):
            assert_pending_pdf_text(pdf_origins(original, unclipped),
                                    pdf_origins(painted, unclipped), name)
        compare_paragraph_backgrounds(checked_file(root, case['source']), original, painted, name)
        planes = {}
        for plane in ('native', 'visible'):
            actual = read_json(target / (plane + '.json'))
            glyphs = [g for g in actual['glyphs'] if not g[1].isspace()]
            require(actual['input_sha256'] == case['source']['sha256'], 'Wrong rendered legacy input')
            require(actual['page_count'] == models[name]['page_count']
                    and [g[:2] for g in glyphs] == [g[:2] for g in models[name]['glyphs']],
                    name + ': native/live text, pages or order changed')
            require(all(math.isfinite(g[axis]) for g in glyphs for axis in (2, 3)),
                    name + ': nonfinite native/live coordinate')
            if name in xps_required:
                planes[plane] = assert_model(models[name], actual, name + '/XPS/' + plane, True)
        results.append(dict(name=name, required_font=name in font_required,
                            required_xps=name in xps_required, planes=planes))
    require(len(results) == 80 and sum(r['required_font'] for r in results) == 68
            and sum(r['required_xps'] for r in results) == 66, 'Required legacy assertions removed')
    print('Asserted 66 legacy XPS sources on both paths; 68 required font four-plane controls; '
          'all 80 text/page/order/finite and paragraph backgrounds, including 12 pending inputs')
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compare-rendered-fonts', type=Path)
    parser.add_argument('--native-executable', type=Path)
    args = parser.parse_args()
    if args.compare_rendered_fonts:
        check_rendered_fonts(args.compare_rendered_fonts.resolve(),
                             executable=args.native_executable.resolve() if args.native_executable else None)
    else:
        print('Verified', len(verify()[0]), 'versioned original Word legacy controls; '
              '8 incomplete subset-font inputs and 4 modern manual-break differences remain pending')
