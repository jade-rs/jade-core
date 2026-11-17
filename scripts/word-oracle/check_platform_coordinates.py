"""Assert shared DOCX identity and independent Mac/Windows Word expectations.

Mac rendering is checked by check_mac_word_coordinates.mjs on macOS. This
registry also runs the Windows native renderer against every Windows capture,
including raw EMF, PDF text and independently painted native PDF origins.
"""
import argparse
from collections import Counter, defaultdict, deque
import gzip
import hashlib
import json
import math
from pathlib import Path
import subprocess
import sys
from zipfile import ZipFile

from extract_windows_emf import extract, emf_glyphs

ROOT = Path(__file__).resolve().parents[2]
INDEX = Path('tests/word-oracle/platform-coordinates.json')
REQUIRED_CAPTURES = {
    'aptos-ascii-advances-mac': '20261117T123854554Z-f62dd6d1',
    'floating-square-wrap-mac': '20261114T004143028Z-b4cb9887',
    'floating-wrap-sides-mac': '20261114T004143028Z-b4cb9887',
    'word-mac-probe-floating-horizontal-15': '20261114T015622405Z-203d1791',
    'probe-hyphenation-split-word-sweep-off': '20261114T020157033Z-88c4d52e',
    'probe-contextual-paragraph-spacing-mode14': '20261114T023153447Z-6c24de77',
    'probe-contextual-paragraph-spacing-mode15': '20261114T023156058Z-d477ae1f',
    'probe-paragraph-border-wrap': '20261114T023201516Z-623785ec',
    'endnote-mac': '20261114T004143028Z-b4cb9887',
    'footnote-long-continuation-mac': '20261114T004143028Z-b4cb9887',
    'footnote-table-textbox-order-mac': '20261114T004143028Z-b4cb9887',
    'footnote-mac': '20261114T004143028Z-b4cb9887',
    'footnote-table-each-page-mac': '20261114T004143028Z-b4cb9887',
    'align-center-mac': '20261114T020438767Z-bfed5b87',
    'alignment-mac': '20261114T020441411Z-af6ea560',
    'aptos-range-compat-14-en-US-mac': '20261114T004240145Z-5f9102be',
    'ascii-punctuation-mac': '20261114T004252975Z-a999e8fb',
    'cell-alignment-mac': '20261114T004252975Z-a999e8fb',
    'column-break-mac': '20261114T004252975Z-a999e8fb',
    'decimal-tabs-mac': '20261114T004252975Z-a999e8fb',
    'direction-with-selection-mac': '20261114T004143028Z-b4cb9887',
    'eqruby-grid-mac': '20261114T004202539Z-7cf29738',
    'eqruby-nogrid-mac': '20261114T004202539Z-7cf29738',
    'fractional-picture-anchors-mac': '20261114T020548688Z-7207704a',
    'header-footer-sections-mac': '20261114T004143028Z-b4cb9887',
    'header-tab-overflow-long-mac': '20261114T020253083Z-9dcfef94',
    'indents-mac': '20261114T004309406Z-2c388f57',
    'inline-image-position-grid-mac': '20261114T004813814Z-c21deb91',
    'inline-image-position-nogrid-mac': '20261114T004813814Z-c21deb91',
    'inline-image-rotation-table-picture-only-mac': '20261114T004309406Z-2c388f57',
    'jade-script-baseline-20261030': '20261114T004704886Z-7e8168a7',
    'kana-mac': '20261114T004309406Z-2c388f57',
    'keep-mac': '20261114T004309406Z-2c388f57',
    'latin-break-budget-620': '20261114T004309406Z-2c388f57',
    'latin-break-budget-638': '20261114T004309406Z-2c388f57',
    'latin-break-budget-645': '20261114T004309406Z-2c388f57',
    'latin-break-budget-652': '20261114T004309406Z-2c388f57',
    'latin-break-budget-659': '20261114T004309406Z-2c388f57',
    'line-numbering-mac': '20261114T004322231Z-e7a4d03f',
    'line-spacing-mac': '20261114T004322231Z-e7a4d03f',
    'merged-table-over-page-mac': '20261114T005709592Z-772bee19',
    'mirror-margins-mac': '20261114T004322231Z-e7a4d03f',
    'mirror-margins-parity-mac': '20261114T020311702Z-0fc768f7',
    'nested-after-spacing-oversized-48-0': '20261114T004322231Z-e7a4d03f',
    'nested-after-spacing-repeat-header': '20261114T004322231Z-e7a4d03f',
    'nested-after-spacing-short-5pt': '20261114T004334255Z-b74a0cd3',
    'nested-merged-tables-mac': '20261114T004334255Z-b74a0cd3',
    'note-styles-creation-mac': '20261114T020313084Z-50588e4b',
    'numbering-by-style-mac': '20261114T020314478Z-267eae10',
    'numbering-restart-mac': '20261114T020324309Z-57157ab7',
    'odd-even-stories-mac': '20261114T004334255Z-b74a0cd3',
    'paragraph-spacing-mac': '20261114T004346218Z-5f779447',
    'probe-unequal-left-narrow-first-1-1': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-left-narrow-first-2-1': '20261114T004547596Z-7db3b26c',
    'probe-unequal-left-wide-first-1-1': '20261114T004547596Z-7db3b26c',
    'probe-unequal-left-wide-first-2-1': '20261114T004547596Z-7db3b26c',
    'ruby-grid-mac': '20261114T004214564Z-4ebc9049',
    'ruby-nogrid-mac': '20261114T004214564Z-4ebc9049',
    'ruby-wide-grid-mac': '20261114T004214564Z-4ebc9049',
    'ruby-wide-nogrid-mac': '20261114T004214564Z-4ebc9049',
    'run-styles-mac': '20261114T020737875Z-7d16155a',
    'script-position-mac': '20261114T020739586Z-407e2e78',
    'section-break-types-mac': '20261114T021536775Z-818619fb',
    'sections-mac': '20261114T020750373Z-ad48fa89',
    'story-inline-picture-metrics-mac': '20261114T020753623Z-4d5f0b2b',
    'table-boundary-cant-split-mac': '20261114T004633221Z-54077165',
    'table-oversized-row-after-spacing-fresh-fit': '20261114T020835626Z-17c9262c',
    'table-row-after-spacing-short-5pt': '20261114T004647940Z-877c5941',
    'table-vertical-merge-page-break-mac': '20261114T004202539Z-7cf29738',
    'table3x2merged-mac': '20261114T004647940Z-877c5941',
    'tabs-mac': '20261114T004647940Z-877c5941',
    'textbox-exact-line-matrix-mac': '20261114T004647940Z-877c5941',
    'textbox-fixed-pitch-matrix-mac': '20261114T004647940Z-877c5941',
    'toc-sequence-page-number-mac': '20261114T004202539Z-7cf29738',
    'underline-kinds-mac': '20261114T004202539Z-7cf29738',
    'unequal-columns-mac': '20261114T004647940Z-877c5941',
    'vert-align-sweep-2-browser-mac': '20261114T020406409Z-43ea3b48',
    'vert-align-sweep-browser-mac': '20261114T004647940Z-877c5941',
    'widow-control-mac': '20261114T004704886Z-7e8168a7',
    'word-mac-probe-distribute-single': '20261114T004202539Z-7cf29738',
    'word-mac-probe-indent': '20261114T004214564Z-4ebc9049',
    'word-mac-probe-numbering-label': '20261114T004214564Z-4ebc9049',
    'word-mac-probe-numbering-label-faces': '20261114T004214564Z-4ebc9049',
    'word-mac-probe-object-space-runs-unbalanced': '20261114T004214564Z-4ebc9049',
    'word-mac-probe-squeeze-conditions-nocompress': '20261114T020348514Z-150cc80a',
    'word-mac-probe-squeeze-plain': '20261114T020357360Z-c1ab8e03',
    'word-mac-probe-vert-align-sweep-2': '20261114T004704886Z-7e8168a7',
    'wrong-num-mac': '20261114T020410811Z-30c286b3',
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read_json(path):
    return json.loads(path.read_text(encoding='utf-8-sig'))


def repo_file(root, relative):
    require(isinstance(relative, str) and '\\' not in relative and ':' not in relative
            and not Path(relative).is_absolute(),
            'Expected repository-relative POSIX path')
    path = (root / relative).resolve()
    require(path.is_relative_to(root.resolve()), 'Path escapes repository')
    require(path.is_file(), f'Missing evidence: {relative}')
    return path


def checked_file(root, record):
    path = repo_file(root, record['file'])
    require(hashlib.sha256(path.read_bytes()).hexdigest() == record['sha256'],
            f'Evidence changed: {record["file"]}')
    return path


def ordered(glyphs):
    result = sorted((g for g in glyphs if not g[1].isspace()), key=lambda g: (g[0], g[3]))
    start = 0
    while start < len(result):
        end = start + 1
        while (end < len(result) and result[end][0] == result[start][0]
               and abs(result[end][3] - result[start][3]) <= .25):
            end += 1
        result[start:end] = sorted(result[start:end], key=lambda g: g[2])
        start = end
    return result


def compare(expected, actual, label):
    require(actual['page_count'] == expected['page_count'], f'{label}: page count')
    expected_glyphs = [g for g in expected['glyphs'] if not g[1].isspace()]
    actual_glyphs = [g for g in actual['glyphs'] if not g[1].isspace()]
    require(bool(expected_glyphs), f'{label}: empty reference')
    require(len(expected_glyphs) == len(actual_glyphs), f'{label}: missing/extra glyphs')
    require(Counter(tuple(g[:2]) for g in expected_glyphs)
            == Counter(tuple(g[:2]) for g in actual_glyphs),
            f'{label}: text or page changed')
    # Floating text and a superscript can be only 0.24pt apart vertically.
    # Grouping baselines at 0.25pt changes reading order after harmless Word
    # rounding. Match the same character/page at its actual XY origin instead;
    # every origin still needs a distinct partner within 0.375pt on BOTH axes.
    tolerance = .375
    buckets = defaultdict(list)
    for i, glyph in enumerate(actual_glyphs):
        require(all(math.isfinite(glyph[axis]) for axis in (2, 3)),
                f'{label}/{i}: nonfinite axis')
        buckets[(glyph[0], glyph[1], math.floor(glyph[2] / tolerance),
                 math.floor(glyph[3] / tolerance))].append(i)
    candidates = []
    for i, word in enumerate(expected_glyphs):
        x, y = math.floor(word[2] / tolerance), math.floor(word[3] / tolerance)
        nearby = [j for dx in (-1, 0, 1) for dy in (-1, 0, 1)
                  for j in buckets.get((word[0], word[1], x + dx, y + dy), ())
                  if all(abs(word[axis] - actual_glyphs[j][axis]) <= tolerance
                         for axis in (2, 3))]
        require(bool(nearby), f'{label}/{i}: axis mismatch: Word {word}')
        nearby.sort(key=lambda j: sum((word[axis] - actual_glyphs[j][axis]) ** 2
                                      for axis in (2, 3)))
        candidates.append(nearby)
    word_match, jade_match = {}, {}
    for start in sorted(range(len(candidates)), key=lambda i: len(candidates[i])):
        queue, seen, predecessor = deque([start]), {start}, {}
        free = None
        while queue and free is None:
            word_index = queue.popleft()
            for jade_index in candidates[word_index]:
                if jade_index in predecessor:
                    continue
                predecessor[jade_index] = word_index
                if jade_index not in jade_match:
                    free = jade_index
                    break
                other = jade_match[jade_index]
                if other not in seen:
                    seen.add(other)
                    queue.append(other)
        require(free is not None, f'{label}/{start}: missing distinct glyph at expected axes')
        while free is not None:
            word_index = predecessor[free]
            previous = word_match.get(word_index)
            word_match[word_index], jade_match[free] = free, word_index
            free = previous
    return len(expected_glyphs)


def pdf_origins(path, unclipped=False):
    import pymupdf
    flags = pymupdf.TEXTFLAGS_RAWDICT
    if unclipped:
        flags &= ~pymupdf.TEXT_CLIP
    with pymupdf.open(path) as document:
        glyphs = []
        for number, page in enumerate(document, 1):
            for block in page.get_text('rawdict', flags=flags)['blocks']:
                for line in block.get('lines', []):
                    for span in line['spans']:
                        glyphs.extend([number, char['c'], *char['origin']]
                                      for char in span['chars'] if not char['c'].isspace())
        return dict(page_count=len(document), glyphs=glyphs)


def extract_fonts(pdf, directory):
    """Expose original font programs, never Word's expected text positions."""
    import pymupdf
    directory.mkdir(parents=True, exist_ok=False)
    with pymupdf.open(pdf) as document:
        xrefs = {font[0] for page in document for font in page.get_fonts(full=True)}
        count = 0
        for xref in sorted(xrefs):
            _, extension, _, data = document.extract_font(xref)
            if data and extension in ('ttf', 'otf', 'ttc'):
                (directory / f'{xref}.{extension}').write_bytes(data)
                count += 1
    require(count > 0, 'Original Word PDF contains no usable embedded fonts')


def archive_reference(root, case, windows):
    """Read one document from an immutable, possibly multi-document capture.

    Drawing origins are decoded from original EMFs in memory. The archive and
    capture metadata remain byte-for-byte original; no selected capture or
    renderer-derived expected positions are generated.
    """
    archive = checked_file(root, windows['archive'])
    with ZipFile(archive) as bundle:
        members = bundle.namelist()
        require(len(members) == len(set(members)) and all(
            not Path(name).is_absolute() and ':' not in name and '\\' not in name
            and '..' not in Path(name).parts for name in members),
            'Invalid or duplicate original archive member')
        raw = bundle.read('capture.json')
        require(hashlib.sha256(raw).hexdigest() == windows['capture_sha256'],
                'Original archived capture metadata changed')
        capture = json.loads(raw.decode('utf-8-sig'))
        require(capture['schema'] == 2 and capture['platform'] == 'windows'
                and capture['status'] == 'complete', 'Incomplete or mislabelled Windows capture')
        require(capture['capture_id'] == windows['capture_id'], 'Archived capture identity changed')
        require(capture['word']['product_version'] == windows['word_product_version'],
                'Windows Word version changed')
        require(capture['word']['executable_sha256'] == windows['word_executable_sha256'],
                'Windows Word executable changed')
        require(archive.parent.name == 'word-' + windows['word_product_version'],
                'Wrong archived Word version directory')
        documents = [d for d in capture['documents'] if d['name'] == windows['fixture']]
        require(len(documents) == 1, 'Missing or duplicate archived Windows input')
        document = documents[0]
        # Older captures predate this field. Keep their missing provenance;
        # never relabel them as a newer capture or accept a recorded repair.
        require(document.get('open_mode') in (None, 'normal-read-only'),
                'Archived input was repaired or converted')
        require(document['sha256'] == case['source']['sha256'], 'Windows source identity changed')
        source = checked_file(root, case['source'])
        require(bundle.read('inputs/' + document['name'] + '.docx') == source.read_bytes(),
                'Windows input differs from Mac original')
        require(hashlib.sha256(bundle.read(document['pdf'])).hexdigest() == document['pdf_sha256'],
                'Original archived Windows Word PDF changed')
        pages = document['metafiles']
        require([p['page'] for p in pages] == list(range(1, document['pages'] + 1)),
                'Archived Word page coverage changed')
        glyphs = []
        for page in pages:
            data = bundle.read(page['file'])
            require(hashlib.sha256(data).hexdigest() == page['sha256'],
                    'Original archived Windows Word EMF changed')
            glyphs.extend(emf_glyphs(data, page['page'], page['width_pt'], page['height_pt']))
        drawing = dict(page_count=document['pages'], glyphs=glyphs)
        require(capture.get('table_glyph_projection', 'visible') in ('visible', 'unclipped'),
                'Unknown archived drawing projection')
        require(windows.get('font_source') == 'original-word-pdf', 'Unknown archived font source')
    return archive, capture, document, drawing


def verify(root=ROOT):
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and bool(index['fixtures']), 'Empty platform registry')
    names = set()
    cases = []
    for case in index['fixtures']:
        name = case['mac_case']
        require(name not in names, 'Duplicate shared fixture')
        names.add(name)
        directory = f'tests/word-oracle/mac-elsa-synthetic/{name}'
        source = checked_file(root, case['source'])
        require(case['source']['file'] == f'{directory}/source.docx', 'Wrong Mac source')
        mac_file = checked_file(root, case['mac_manifest'])
        require(case['mac_manifest']['file'] == f'{directory}/manifest.json', 'Wrong Mac manifest')
        mac = read_json(mac_file)
        require(mac['reference']['platform'] == 'mac', 'Mac reference platform changed')
        # Some imported Mac captures predate version recording. Preserve that
        # provenance gap instead of inventing a build or changing their goldens.
        require(mac['reference']['application'] == 'Microsoft Word', 'Wrong Mac application')
        require(mac['sourceSha256'] == case['source']['sha256'], 'Mac source identity changed')
        coordinates = repo_file(root, f'{directory}/pages.json.gz').read_bytes()
        require(hashlib.sha256(coordinates).hexdigest() == mac['coordinatesSha256'],
                'Mac coordinates changed')
        require(len(json.loads(gzip.decompress(coordinates))) == mac['pages'], 'Mac page count')
        require(bool(case['windows']), 'Missing Windows version expectations')
        captures = set()
        for windows in case['windows']:
            if 'archive' in windows:
                require('capture' not in windows and 'drawing_reference' not in windows,
                        'Ambiguous original Windows reference')
                archive, capture, document, fixture = archive_reference(root, case, windows)
                key = (capture['word']['product_version'], capture['capture_id'])
                require(key not in captures, 'Duplicate Word version/capture')
                captures.add(key)
                cases.append((name, source, archive, document, fixture, 'original-word-pdf'))
                continue
            manifest = checked_file(root, windows['capture'])
            capture = read_json(manifest)
            key = (capture['word']['product_version'], capture['capture_id'])
            require(key not in captures, 'Duplicate Word version/capture')
            captures.add(key)
            require(capture['platform'] == 'windows' and capture['status'] == 'complete',
                    'Incomplete or mislabelled Windows capture')
            require(key[0] == windows['word_product_version'], 'Windows Word version changed')
            documents = [d for d in capture['documents'] if d['name'] == windows['fixture']]
            require(len(documents) == 1, 'Missing or duplicate Windows input')
            document = documents[0]
            require(document['sha256'] == case['source']['sha256'], 'Windows source identity changed')
            archived = manifest.parent / 'inputs' / (document['name'] + '.docx')
            require(archived.read_bytes() == source.read_bytes(), 'Windows input differs from Mac original')
            require(hashlib.sha256((manifest.parent / document['pdf']).read_bytes()).hexdigest()
                    == document['pdf_sha256'], 'Original Windows Word PDF changed')
            reference_file = checked_file(root, windows['drawing_reference'])
            require(reference_file.parent == manifest.parent, 'Wrong drawing capture')
            reference = read_json(reference_file)
            require(reference == extract(manifest), 'Drawing origins differ from original Word EMFs')
            require(reference['platform'] == 'windows' and reference['tolerance_pt'] == .375,
                    'Windows platform/tolerance changed')
            fixture = next(f for f in reference['fixtures'] if f['name'] == document['name'])
            require(fixture['docx_sha256'] == case['source']['sha256'], 'Wrong drawing input')
            font_source = windows.get('font_source', 'installed')
            require(font_source in ('installed', 'original-word-pdf'), 'Unknown font source')
            cases.append((name, source, manifest, document, fixture, font_source))
        if name in REQUIRED_CAPTURES:
            require(('16.0.19127.20936', REQUIRED_CAPTURES[name]) in captures,
                    'Previously asserted Word version/capture was removed')
    require(REQUIRED_CAPTURES.keys() <= names, 'Shared fixture coverage removed')
    return cases


def check_windows(executable, output, root=ROOT):
    require(sys.platform == 'win32', 'Windows font measurements require Windows')
    cases = verify(root)
    output.mkdir(parents=True, exist_ok=True)
    checked = 0
    for name, source, manifest, document, drawing, font_source in cases:
        archived = manifest.suffix == '.zip'
        if archived:
            with ZipFile(manifest) as bundle:
                capture = json.loads(bundle.read('capture.json').decode('utf-8-sig'))
                original_pdf_bytes = bundle.read(document['pdf'])
        else:
            capture = read_json(manifest)
        target = output / name / capture['capture_id']
        target.mkdir(parents=True, exist_ok=True)
        actual_path, painted_path = target / 'native.json', target / 'native.pdf'
        if archived:
            original_pdf = target / 'original-word.pdf'
            require(not original_pdf.exists(), 'Refusing to replace extracted original PDF')
            original_pdf.write_bytes(original_pdf_bytes)
        else:
            original_pdf = manifest.parent / document['pdf']
        options = []
        if font_source == 'original-word-pdf':
            extract_fonts(original_pdf, target / 'fonts')
            options = ['--fonts-directory', str(target / 'fonts')]
        subprocess.run([str(executable), str(source), str(actual_path), 'windows',
                        str(painted_path), '--unclipped-tables', *options], check=True)
        actual = read_json(actual_path)
        compare(pdf_origins(original_pdf, True), actual, name + '/PDF-unclipped')
        visible_path = target / 'visible.json'
        subprocess.run([str(executable), str(source), str(visible_path), 'windows', *options], check=True)
        visible = pdf_origins(original_pdf)
        actual_visible = read_json(visible_path)
        for model in (actual, actual_visible):
            require(model['input_sha256'] == hashlib.sha256(source.read_bytes()).hexdigest(),
                    'Wrong shared rendered input')
            if font_source == 'original-word-pdf':
                from check_windows_font_coordinates import require_original_font_programs_registered
                require_original_font_programs_registered(target / 'fonts', model)
        projection = capture.get('table_glyph_projection', 'visible')
        checked += compare(drawing, actual if projection == 'unclipped' else actual_visible,
                           name + '/EMF')
        compare(visible, actual_visible, name + '/PDF-visible')
        require(painted_path.read_bytes() != original_pdf.read_bytes(), 'Word self-comparison')
        compare(visible, pdf_origins(painted_path), name + '/painted-PDF')
        print(f'{name}: {document["pages"]} pages, {len(drawing["glyphs"])} glyphs, '
              f'Windows Word {capture["word"]["product_version"]}: PASS', flush=True)
    return checked


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/platform-word-coordinates')
    args = parser.parse_args()
    if args.native_executable:
        print(f'Checked {check_windows(args.native_executable.resolve(), args.output_directory.resolve())} drawing origins')
    else:
        print(f'Verified {len(verify())} shared DOCX/platform-version reference pairs')
