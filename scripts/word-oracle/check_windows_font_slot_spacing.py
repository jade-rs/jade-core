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
import generate_windows_aptos_spacing_probes as producer
import generate_windows_legacy_mode_balance_probes as legacy_producer
import generate_windows_times_script_slot_probes as slot_producer
import generate_windows_times_japanese_spacing_probes as japanese_producer
import generate_windows_times_line_end_probes as line_end_producer
import generate_windows_legacy_mincho_bold_probes as long_bold_producer
import generate_windows_mincho_context_probes as context_producer
import generate_windows_mincho_font_switch_probes as switch_producer
import generate_windows_mincho_prefix_probes as prefix_producer
import generate_windows_times_balanced_space_probes as space_producer
import generate_windows_times_balanced_space_mode_probes as space_mode_producer
import generate_windows_gdi_bold_justification_probes as gdi_producer
import generate_windows_gdi_bold_natural_probes as natural_producer
import generate_windows_origin_phase_probes as phase_producer
import generate_windows_origin_tie_probes as tie_producer
from itertools import chain

INDEX = Path('tests/word-oracle/windows/font-slot-spacing-observations.json')
INDEX_SHA256 = '260a1f6876aab157823633788d68a6bdac2b76c7dc8fb1002144a4f48f20428d'
FONT_NAMES_SHA256 = '3daa9b320b441260cf05dac1d8371c3169b41b913de227d4976fd5f94f297f3b'
XPS_NAMES_SHA256 = '3daa9b320b441260cf05dac1d8371c3169b41b913de227d4976fd5f94f297f3b'
REQUIRED_CAPTURES = {'20261119T085721179Z-ad4e1a41': 24, '20261119T090056485Z-6c01f12e': 32, '20261119T090823813Z-a9ddf686': 96, '20261119T092533754Z-02a811e6': 48, '20261119T102627743Z-e19cdbe7': 16, '20261119T112518348Z-0dd586f2': 40, '20261119T114842423Z-5f6babdf': 20, '20261119T123214849Z-2db788c9': 8, '20261119T125532465Z-61d0e4bd': 10, '20261119T130101255Z-42e608fb': 8, '20261119T132210919Z-158142e9': 4, '20261119T133344101Z-1b09b6f8': 8, '20261119T133746583Z-14920783': 16, '20261119T135242965Z-0dd32fe2': 16, '20261119T140807475Z-2aee0646': 32, '20261119T142153366Z-edd25613': 32, '20261119T143721831Z-2fe0958f': 16, '20261119T145753302Z-cf8cad83': 8}


def locked_promotions(index):
    rows = index['fixtures']
    require(index['schema'] == 1 and len(rows) == len({r['name'] for r in rows}) == 434,
            'Font-slot spacing source coverage changed')
    names = []
    for field, promotion, count, digest in (
            ('font_gate_promotion', 'font-slot-spacing-font', 434, FONT_NAMES_SHA256),
            ('xps_gate_promotion', 'font-slot-spacing-xps', 434, XPS_NAMES_SHA256)):
        selected = sorted(r['name'] for r in rows if r[field] == promotion)
        require(len(selected) == count and hashlib.sha256(
            json.dumps(selected, separators=(',', ':')).encode()).hexdigest() == digest,
            'Required mixed-script font-slot spacing coordinate assertions changed')
        require(all(r[field] in (promotion, 'pending') for r in rows), 'Unknown mixed-script font-slot spacing promotion')
        names.append(frozenset(selected))
    require(names[1] <= names[0], 'Required XPS output missing a font render')
    return tuple(names)


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Original mixed-script font-slot spacing registry changed')
    index = read_json(root / INDEX)
    required, xps_required = locked_promotions(index)
    generated = {n: (data, spec) for n, data, spec in chain(producer.generate(root), legacy_producer.generate(root), slot_producer.generate(root), japanese_producer.generate(root), line_end_producer.generate(root), long_bold_producer.generate(root), context_producer.generate(root), switch_producer.generate(root), prefix_producer.generate(root), space_producer.generate(root), space_mode_producer.generate(root), gdi_producer.generate(root), natural_producer.generate(root), phase_producer.generate(root), tie_producer.generate(root))}
    require(set(generated) == {r['name'] for r in index['fixtures']}, 'Font-slot spacing input coverage changed')
    require(index['word']['product_version'] == '16.0.19127.20936'
            and index['word']['executable_sha256'] ==
            '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312',
            'Original mixed-script font-slot spacing Word version/executable changed')
    captures, documents, drawings, xps_models, authoring_facts = {}, {}, {}, {}, {}
    require(len(index['captures']) == 18, 'Font-slot spacing capture coverage changed')
    extras = {'input-facts.json', 'generate-inputs.py', 'capture-wrapper.ps1',
              'capture-word-windows.ps1', 'word-windows-page-geometry.ps1',
              'word-output-font-facts.json', 'capture-provenance.json'}
    for record in index['captures']:
        manifest = checked_file(root, record['capture'])
        capture = read_json(manifest)
        require(capture['schema'] == 2 and capture['status'] == 'complete'
                and capture['platform'] == 'windows' and capture['word'] == index['word']
                and capture['capture_id'] == record['capture_id'], 'Original mixed-script font-slot spacing capture identity changed')
        count = len(capture['documents'])
        require(count == record['documents'] == REQUIRED_CAPTURES.get(capture['capture_id']), 'Font-slot spacing document coverage changed')
        with ZipFile(checked_file(root, record['archive'])) as archive, \
                ZipFile(checked_file(root, record['original_capture'])) as original:
            members = set(archive.namelist())
            require(len(archive.namelist()) == len(members) == 4 * count + 1
                    and len(original.namelist()) == len(set(original.namelist())) == 4 * count + 8
                    and set(original.namelist()) == members | extras,
                    'Original mixed-script font-slot spacing archive coverage changed')
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
            dependencies = provenance['generation_dependencies']
            require(dependencies['repo'] == 'jade-rs/jade-core'
                    and dependencies['head'] == 'dd5fbc9a0a9d749aca16e7f33082b2fbbc946d83',
                    'Original mixed-face generation dependency identity changed')
            expected_dependencies = {'tests/word-oracle/windows/legacy-justification-observations.json'}
            if capture['capture_id'] in ('20261119T085721179Z-ad4e1a41', '20261119T090056485Z-6c01f12e'):
                expected_dependencies.add('scripts/word-oracle/generate_windows_legacy_justification_probes.py')
            if capture['capture_id'] == '20261119T125532465Z-61d0e4bd':
                expected_dependencies = {p for p, _ in long_bold_producer.PARENTS.values()}
            require(set(dependencies['files_sha256']) == expected_dependencies,
                    'Original generation dependency coverage changed')
            for relative, digest in dependencies['files_sha256'].items():
                data = (root / relative).read_bytes()
                require(hashlib.sha256(data).hexdigest() == digest
                        or relative.endswith('.py') and hashlib.sha256(
                            data.replace(b'\r\n', b'\n')).hexdigest() == digest,
                        'Original generation dependency changed beyond line endings')
            authored = {r['name']: r for r in json.loads(original.read('input-facts.json'))}
            require(len(authored) == count, 'Original authoring source coverage changed')
            authoring_facts.update(authored)
            require(set(provenance['tools_sha256']) == extras - {'word-output-font-facts.json', 'capture-provenance.json'},
                    'Original capture tool coverage changed')
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
                        'Original mixed-script font-slot spacing source/open mode changed')
                for key in ('pdf', 'xps'):
                    path = manifest.parent / document[key]
                    require(path.read_bytes() == archive.read(document[key])
                            and hashlib.sha256(path.read_bytes()).hexdigest() == document[key + '_sha256'],
                            'Original mixed-script font-slot spacing PDF/XPS changed')
                font_facts = facts[name]
                require(font_facts['word'] == capture['word'] and font_facts['source_sha256'] == document['sha256'],
                        'Original mixed-script font-slot spacing font identity changed')
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
    require(set(captures) == set(REQUIRED_CAPTURES), 'Original capture batches removed/substituted')
    require(len(documents) == 434, 'Font-slot spacing originals removed')
    fonts = {c['name']: c for c in read_json(root / 'tests/word-oracle/windows/font-coordinates.json')['fixtures']}
    for case in index['fixtures']:
        name = case['name']
        record, document, manifest = documents[name]
        data, specification = generated[name]
        authored = authoring_facts[name]
        require(authored['source_sha256'] == case['source']['sha256']
                and {k: v for k, v in authored.items() if k not in
                     ('name', 'source', 'source_sha256', 'width_probe')} == specification
                and case.get('authoring_width_probe') == authored.get('width_probe'),
                'Original mixed-face source/width authoring facts changed')
        require(checked_file(root, case['source']).read_bytes() == data
                and case['input_specification'] == specification
                and case['capture'] == record['capture'] and case['archive'] == record['archive']
                and case['capture_id'] == record['capture_id'] and case['word'] == index['word'],
                'Font-slot spacing original specification/reference changed')
        wanted = specification['expected_non_whitespace_glyphs']
        drawing = drawings[name]
        xps = pdf_origins(manifest.parent / document['xps'], True)
        require(drawing['page_count'] == xps['page_count'] == document['pages'] == 1
                and len(drawing['glyphs']) == case['glyph_count']
                and sum(not g[1].isspace() for g in drawing['glyphs']) == len(xps['glyphs']) == wanted,
                'Font-slot spacing page/glyph coverage changed')
        xps_models[name] = xps
        if name in required:
            require(case.get('font_program_source') == fonts.get(name, {}).get('font_program_source'),
                    'Wrong mixed-script font-slot spacing original font source')
            require(name in fonts and fonts[name] == dict(name=name, capture=case['capture'],
                drawing_reference=record['drawing_reference'], source=case['source'],
                word_product_version=index['word']['product_version'], glyph_count=case['glyph_count'],
                public_test_password=None, solid_paragraph_background=True,
                **({'font_program_source': dict(kind='original-xps',
                    file=(manifest.parent / document['xps']).relative_to(root).as_posix(),
                    sha256=document['xps_sha256'])} if 'font_program_source' in case else {})),
                'Required mixed-script font-slot spacing font assertions removed/substituted')
        else:
            require(name not in fonts, 'Pending mixed-face mixed-script font-slot spacing coordinates promoted without proof')
    require(sum('font_program_source' in r for r in index['fixtures']) == 434
            and all('pending_font_reason' not in r and 'pending_xps_reason' not in r
                    for r in index['fixtures']), 'Resolved font-input coverage changed')
    return index['fixtures'], required, xps_required, xps_models, documents


def assert_rendered_case(case, directory, expected_xps, document, manifest, root=ROOT):
    from check_platform_coordinates import assert_pending_pdf_text
    from paragraph_backgrounds import compare_paragraph_backgrounds
    name = case['name']
    target = directory / case['capture_id'] / name
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
        require(actual['input_sha256'] == case['source']['sha256'], 'Wrong rendered mixed-face input')
        require(actual['page_count'] == expected_xps['page_count']
                and [g[:2] for g in glyphs] == [g[:2] for g in expected_xps['glyphs']],
                name + ': native/live text, pages or order changed')
        require(all(math.isfinite(g[axis]) for g in glyphs for axis in (2, 3)),
                name + ': nonfinite native/live coordinate')
        planes[plane] = assert_model(expected_xps, actual, name + '/XPS/' + plane, True)
    return planes


def check_rendered_fonts(directory, root=ROOT):
    cases, font_required, xps_required, models, documents = verify(root)
    results = []
    for case in cases:
        name = case['name']
        _, document, manifest = documents[name]
        planes = assert_rendered_case(case, directory, models[name], document, manifest, root)
        results.append(dict(name=name, required_font=name in font_required,
                            required_xps=name in xps_required, planes=planes))
    require(len(results) == 434 and sum(r['required_font'] for r in results) == 434
            and sum(r['required_xps'] for r in results) == 434, 'Required mixed-face assertions removed')
    print('Asserted 434 font-slot XPS sources on both paths; 434 required font four-plane controls; '
          'all 434 text/page/order/finite and paragraph backgrounds, all sources required')
    return results


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compare-rendered-fonts', type=Path)
    args = parser.parse_args()
    if args.compare_rendered_fonts:
        check_rendered_fonts(args.compare_rendered_fonts.resolve())
    else:
        print('Verified', len(verify()[0]), 'versioned original Word mixed-face controls; '
              'all 434 require font and dual-path XPS assertions')
