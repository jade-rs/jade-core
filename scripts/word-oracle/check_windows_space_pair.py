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
import generate_windows_space_pair_probes as producer

INDEX = Path('tests/word-oracle/windows/space-pair-observations.json')
INDEX_SHA256 = '240b2b1161021a9aa39a7aa5a43efc565db060bbee40b79e927ce67ba1b5709e'
FONT_NAMES_SHA256 = '56441984449251e87ee22db4778e7ff3fae1c530abf5f91896de4a3a9a980908'
XPS_NAMES_SHA256 = '56441984449251e87ee22db4778e7ff3fae1c530abf5f91896de4a3a9a980908'


def locked_promotions(index):
    rows = index['fixtures']
    require(index['schema'] == 1 and len(rows) == len({r['name'] for r in rows}) == 32,
            'Space-pair kerning source coverage changed')
    names = []
    for field, promotion, count, digest in (
            ('font_gate_promotion', 'space-pair-font', 32, FONT_NAMES_SHA256),
            ('xps_gate_promotion', 'space-pair-xps', 32, XPS_NAMES_SHA256)):
        selected = sorted(r['name'] for r in rows if r[field] == promotion)
        require(len(selected) == count and hashlib.sha256(
            json.dumps(selected, separators=(',', ':')).encode()).hexdigest() == digest,
            'Required space-pair kerning coordinate assertions changed')
        require(all(r[field] in (promotion, 'pending') for r in rows), 'Unknown space-pair kerning promotion')
        names.append(frozenset(selected))
    require(names[1] <= names[0], 'Required XPS output missing a font render')
    return tuple(names)


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Original space-pair kerning registry changed')
    index = read_json(root / INDEX)
    required, xps_required = locked_promotions(index)
    generated = {n: (data, spec) for n, data, spec in producer.generate(root)}
    require(set(generated) == {r['name'] for r in index['fixtures']}, 'Space-pair kerning input coverage changed')
    require(index['word']['product_version'] == '16.0.19127.20936'
            and index['word']['executable_sha256'] ==
            '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312',
            'Original space-pair kerning Word version/executable changed')
    captures, documents, drawings, xps_models = {}, {}, {}, {}
    require(len(index['captures']) == 1, 'Space-pair kerning capture coverage changed')
    extras = {'input-facts.json', 'generate-inputs.py', 'capture-wrapper.ps1',
              'capture-word-windows.ps1', 'word-windows-page-geometry.ps1',
              'word-output-font-facts.json', 'capture-provenance.json'}
    for record in index['captures']:
        manifest = checked_file(root, record['capture'])
        capture = read_json(manifest)
        require(capture['schema'] == 2 and capture['status'] == 'complete'
                and capture['platform'] == 'windows' and capture['word'] == index['word']
                and capture['capture_id'] == record['capture_id'], 'Original space-pair kerning capture identity changed')
        count = len(capture['documents'])
        require(count == record['documents'] and count == 32, 'Space-pair kerning document coverage changed')
        with ZipFile(checked_file(root, record['archive'])) as archive, \
                ZipFile(checked_file(root, record['original_capture'])) as original:
            members = set(archive.namelist())
            require(len(archive.namelist()) == len(members) == 4 * count + 1
                    and len(original.namelist()) == len(set(original.namelist())) == 4 * count + 8
                    and set(original.namelist()) == members | extras,
                    'Original space-pair kerning archive coverage changed')
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
                        'Original space-pair kerning source/open mode changed')
                for key in ('pdf', 'xps'):
                    path = manifest.parent / document[key]
                    require(path.read_bytes() == archive.read(document[key])
                            and hashlib.sha256(path.read_bytes()).hexdigest() == document[key + '_sha256'],
                            'Original space-pair kerning PDF/XPS changed')
                font_facts = facts[name]
                require(font_facts['word'] == capture['word'] and font_facts['source_sha256'] == document['sha256'],
                        'Original space-pair kerning font identity changed')
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
    require(len(documents) == 32, 'Space-pair kerning originals removed')
    fonts = {c['name']: c for c in read_json(root / 'tests/word-oracle/windows/font-coordinates.json')['fixtures']}
    for case in index['fixtures']:
        name = case['name']
        record, document, manifest = documents[name]
        data, specification = generated[name]
        require(checked_file(root, case['source']).read_bytes() == data
                and case['input_specification'] == specification
                and case['capture'] == record['capture'] and case['archive'] == record['archive']
                and case['capture_id'] == record['capture_id'] and case['word'] == index['word'],
                'Space-pair kerning original specification/reference changed')
        wanted = specification['expected_non_whitespace_glyphs']
        drawing = drawings[name]
        xps = pdf_origins(manifest.parent / document['xps'], True)
        require(drawing['page_count'] == xps['page_count'] == document['pages'] == 1
                and len(drawing['glyphs']) == case['glyph_count']
                and sum(not g[1].isspace() for g in drawing['glyphs']) == len(xps['glyphs']) == wanted,
                'Space-pair kerning page/glyph coverage changed')
        xps_models[name] = xps
        if name in required:
            require(case.get('font_program_source') == fonts.get(name, {}).get('font_program_source'),
                    'Wrong space-pair kerning original font source')
            require(name in fonts and fonts[name] == dict(name=name, capture=case['capture'],
                drawing_reference=record['drawing_reference'], source=case['source'],
                word_product_version=index['word']['product_version'], glyph_count=case['glyph_count'],
                public_test_password=None, solid_paragraph_background=True,
                **({'font_program_source': dict(kind='original-xps',
                    file=(manifest.parent / document['xps']).relative_to(root).as_posix(),
                    sha256=document['xps_sha256'])} if 'font_program_source' in case else {})),
                'Required space-pair kerning font assertions removed/substituted')
        else:
            require(name not in fonts, 'Pending space-pair space-pair kerning coordinates promoted without proof')
    require(sum('font_program_source' in r for r in index['fixtures']) == 32
            and all('pending_font_reason' not in r and 'pending_xps_reason' not in r
                    for r in index['fixtures']), 'Resolved font-input coverage changed')
    return index['fixtures'], required, xps_required, xps_models, documents


def check_rendered_fonts(directory, root=ROOT):
    from check_platform_coordinates import assert_pending_pdf_text
    from paragraph_backgrounds import compare_paragraph_backgrounds
    cases, font_required, xps_required, models, documents = verify(root)
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
            require(actual['input_sha256'] == case['source']['sha256'], 'Wrong rendered space-pair input')
            require(actual['page_count'] == models[name]['page_count']
                    and [g[:2] for g in glyphs] == [g[:2] for g in models[name]['glyphs']],
                    name + ': native/live text, pages or order changed')
            require(all(math.isfinite(g[axis]) for g in glyphs for axis in (2, 3)),
                    name + ': nonfinite native/live coordinate')
            if name in xps_required:
                planes[plane] = assert_model(models[name], actual, name + '/XPS/' + plane, True)
        results.append(dict(name=name, required_font=name in font_required,
                            required_xps=name in xps_required, planes=planes))
    require(len(results) == 32 and sum(r['required_font'] for r in results) == 32
            and sum(r['required_xps'] for r in results) == 32, 'Required space-pair assertions removed')
    print('Asserted 32 space-pair XPS sources on both paths; 32 required font four-plane controls; '
          'all 32 text/page/order/finite and paragraph backgrounds, all sources required')
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compare-rendered-fonts', type=Path)
    args = parser.parse_args()
    if args.compare_rendered_fonts:
        check_rendered_fonts(args.compare_rendered_fonts.resolve())
    else:
        print('Verified', len(verify()[0]), 'versioned original Word space-pair controls; '
              'all 32 require font and dual-path XPS assertions')
