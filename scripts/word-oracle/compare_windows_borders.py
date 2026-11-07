"""Compare monochrome table rules against immutable, versioned Word PDFs.

Compare unions of painted rectangles in both directions, without translation,
scaling, raster thresholds or a permitted failure count. Unsupported black
geometry fails closed. Gray editing guides are outside this monochrome gate.
"""
import argparse
import hashlib
from pathlib import Path
import json
import math
import re
import subprocess
import uuid

import pymupdf

TOLERANCE_PT = 0.375


def rect(value):
    x0, y0, x1, y1 = map(float, value)
    if not all(math.isfinite(c) for c in (x0, y0, x1, y1)):
        raise ValueError('Nonfinite geometry')
    return min(x0, x1), min(y0, y1), max(x0, x1), max(y0, y1)


def intersect(a, b):
    c = max(a[0], b[0]), max(a[1], b[1]), min(a[2], b[2]), min(a[3], b[3])
    return c if c[2] > c[0] + 1e-8 and c[3] > c[1] + 1e-8 else None


def subtract(a, b):
    cut = intersect(a, b)
    if cut is None:
        return [a]
    x0, y0, x1, y1 = a
    l, t, r, b = cut
    return [p for p in [(x0, y0, l, y1), (r, y0, x1, y1),
                       (l, y0, r, t), (l, b, r, y1)]
            if p[2] > p[0] + 1e-8 and p[3] > p[1] + 1e-8]


def black(color):
    return color is not None and len(color) == 3 and all(abs(c) < 1e-6 for c in color)


def rectangles(path):
    result = []
    with pymupdf.open(path) as document:
        for page in document:
            if page.get_images():
                raise ValueError('Raster content cannot serve as a vector border reference')
            clips = []
            groups = []
            rows = []
            for drawing in page.get_drawings(extended=True):
                level = drawing['level']
                clips = [(depth, area) for depth, area in clips if depth < level]
                groups = [(depth, opacity) for depth, opacity in groups if depth < level]
                if drawing['type'] == 'clip':
                    items = drawing['items']
                    if len(items) != 1 or items[0][0] != 're':
                        raise ValueError('Nonrectangular clip requires explicit support')
                    clips.append((level, rect(items[0][1])))
                    continue
                if drawing['type'] == 'group':
                    if (drawing['blendmode'] != 'Normal' or drawing['opacity'] != 1
                            or drawing.get('knockout', False)):
                        raise ValueError('Blended group requires explicit support')
                    groups.append((level, drawing['opacity']))
                    continue
                clip = rect(page.rect)
                for _, area in clips:
                    clip = intersect(clip, area) if clip is not None else None
                if clip is None:
                    continue
                paints = []
                if drawing['fill'] == (1., 1., 1.):
                    if drawing['fill_opacity'] != 1:
                        raise ValueError('Transparent white occlusion requires explicit support')
                    for item in drawing['items']:
                        if item[0] != 're':
                            raise ValueError('White occlusion is not a rectangle')
                        area = intersect(rect(item[1]), clip)
                        if area is not None:
                            rows = [part for previous in rows for part in subtract(previous, area)]
                if black(drawing['fill']):
                    if drawing['fill_opacity'] != 1:
                        raise ValueError('Transparent black fill requires explicit support')
                    for item in drawing['items']:
                        if item[0] != 're':
                            raise ValueError('Black fill is not a rectangle')
                        paints.append(rect(item[1]))
                if black(drawing['color']):
                    if drawing['stroke_opacity'] != 1 or drawing['dashes'] != '[] 0':
                        raise ValueError('Transparent or patterned stroke requires explicit support')
                    if tuple(drawing['lineCap']) != (0, 0, 0):
                        raise ValueError('Non-butt cap requires explicit support')
                    half = drawing['width'] * .5
                    for item in drawing['items']:
                        if item[0] != 'l':
                            raise ValueError('Stroke is not a separate line segment')
                        a, b = item[1:]
                        if abs(a.y - b.y) < 1e-6:
                            paints.append((min(a.x, b.x), a.y-half, max(a.x, b.x), a.y+half))
                        elif abs(a.x - b.x) < 1e-6:
                            paints.append((a.x-half, min(a.y, b.y), a.x+half, max(a.y, b.y)))
                        else:
                            raise ValueError('Non-axis-aligned stroke requires explicit support')
                rows.extend(clipped for paint in paints if (clipped := intersect(paint, clip)))
            result.append(rows)
    return result


def uncovered(source, target, tolerance=TOLERANCE_PT):
    expanded = [(r[0]-tolerance, r[1]-tolerance, r[2]+tolerance, r[3]+tolerance) for r in target]
    failures = []
    for r in source:
        remaining = [r]
        for cover in expanded:
            remaining = [part for area in remaining for part in subtract(area, cover)]
            if not remaining:
                break
        if remaining:
            failures.append(dict(rectangle=r, uncovered=remaining))
    return failures


def compare(word_path, native_path):
    with pymupdf.open(word_path) as word_document, pymupdf.open(native_path) as native_document:
        if len(word_document) != len(native_document):
            raise ValueError('Page count differs')
        for a, b in zip(word_document, native_document):
            if any(abs(x - y) > TOLERANCE_PT for x, y in zip(a.rect, b.rect)):
                raise ValueError('Unscaled page geometry differs')
    word, native = rectangles(word_path), rectangles(native_path)
    if len(word) != len(native):
        raise ValueError('Page count differs')
    result = []
    for page, (a, b) in enumerate(zip(word, native), 1):
        missing, extra = uncovered(a, b), uncovered(b, a)
        result.append(dict(page=page, word_rectangles=len(a), native_rectangles=len(b),
                           word_uncovered_count=len(missing), native_uncovered_count=len(extra),
                           first_missing=missing[:3], first_extra=extra[:3]))
    return result


def checked_path(root, relative):
    path = (root / relative).resolve()
    if not path.is_relative_to(root.resolve()):
        raise ValueError('Reference path escapes its root')
    return path


def verify_hash(path, expected):
    if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
        raise ValueError(f'Immutable source checksum differs: {path}')


def visible_pdf_glyphs(path):
    glyphs = []
    for page_number, page in enumerate(pymupdf.open(path), 1):
        for block in page.get_text('rawdict', flags=pymupdf.TEXTFLAGS_RAWDICT)['blocks']:
            for line in block.get('lines', []):
                for span in line['spans']:
                    glyphs.extend([page_number, char['c'], *char['origin']]
                                  for char in span['chars'] if not char['c'].isspace())
    return glyphs


def compare_visible_text(expected, observed):
    def ordered(glyphs):
        if any(not all(math.isfinite(value) for value in glyph[2:4]) for glyph in glyphs):
            raise ValueError('Nonfinite text geometry')
        glyphs = sorted(glyphs, key=lambda glyph: (glyph[0], glyph[3]))
        start = 0
        while start < len(glyphs):
            end = start + 1
            while end < len(glyphs) and glyphs[end][0] == glyphs[start][0] and abs(glyphs[end][3] - glyphs[start][3]) <= .25:
                end += 1
            glyphs[start:end] = sorted(glyphs[start:end], key=lambda glyph: glyph[2])
            start = end
        return glyphs
    expected, observed = ordered(expected), ordered(observed)
    matching = sum(word[:2] == native[:2] for word, native in zip(expected, observed))
    within = sum(word[:2] == native[:2] and abs(word[2]-native[2]) <= TOLERANCE_PT
                 and abs(word[3]-native[3]) <= TOLERANCE_PT for word, native in zip(expected, observed))
    return dict(word_glyphs=len(expected), native_glyphs=len(observed), matched=matching,
                within_0_375pt=within,
                passed=len(expected) == len(observed) == matching == within)


def run(index_path, executable, output_root):
    root = index_path.resolve().parent
    core = root.parents[2]
    index = json.loads(index_path.read_text(encoding='utf-8-sig'))
    if index['schema'] != 1:
        raise ValueError('Unsupported reference index')
    output = output_root / ('borders-' + uuid.uuid4().hex)
    output.mkdir(parents=True, exist_ok=False)
    results = []
    seen = set()
    required = {
        f'word-windows-edge-table-spacing{kind}-compat{mode}'
        for mode in (12, 14, 15)
        for kind in ('', '-borders', '-continuation', '-auto', '-auto-borders',
                     '-at-least', '-at-least-borders', '-at-least-continuation',
                     '-mixed-borders')
    }
    required.update('word-windows-edge-' + name for name in (
        'table-alignment', 'table-alignment-compat12', 'table-alignment-compat14',
        'table-borders', 'table-borders-compat12', 'table-borders-compat14',
        'table-bidi-compat12', 'table-bidi-compat14', 'table-bidi-compat15',
        'table-indents-compat12', 'table-indents-compat14',
        'table-zero-horizontal-borders-compat12', 'table-zero-horizontal-borders-compat14',
        'table-zero-padding-compat12', 'table-zero-padding-compat14',
    ))
    required.update(f'word-windows-edge-table-{model}-hairline-borders-compat{mode}'
                    for model in ('collapsed', 'spacing') for mode in (12, 14, 15))
    required.update(f'word-windows-edge-table-collapsed-implicit-mixed-borders-compat{mode}'
                    for mode in (12, 14, 15))
    required.update(f'word-windows-edge-table-spacing-{kind}-mixed-borders-compat{mode}'
                    for kind in ('nil', 'auto', 'percent') for mode in (12, 14, 15))
    required.update(f'word-windows-edge-table-{kind}-mixed-borders-compat{mode}'
                    for kind in ('collapsed', 'spacing-missing-width', 'spacing-missing-type')
                    for mode in (12, 14, 15))
    required.update(f'word-windows-edge-table-spacing-negative-mixed-borders-compat{mode}' for mode in (12,14,15))
    required.update(f'word-windows-edge-table-negative-spacing-columns-{count}-compat14' for count in (2,3,4,5))
    required.update(f'word-windows-edge-table-negative-spacing-unequal-{count}-compat14' for count in (2,3,4))
    required.add('word-windows-edge-table-negative-spacing-row-scopes-compat14')
    for entry in index['references']:
        folder = checked_path(root, entry['directory'])
        capture = json.loads((folder / 'capture.json').read_text(encoding='utf-8-sig'))
        reference = json.loads(checked_path(folder, entry['reference']).read_text(encoding='utf-8-sig'))
        version = capture['word']['product_version']
        capture_id = capture['capture_id']
        if (capture['schema'] != 2 or capture['status'] != 'complete'
                or not re.fullmatch(r'\d+\.\d+\.\d+\.\d+', version)
                or not re.fullmatch(r'\d{8}T\d{9}Z-[a-f0-9]{8}', capture_id)
                or folder != root / f'word-{version}' / capture_id
                or version != entry['word_product_version']
                or reference['word_product_version'] != entry['word_product_version']
                or reference['capture_id'] != capture_id
                or reference['word'] != capture['word']):
            raise ValueError('Word capture provenance differs from its registered reference')
        projection = reference.get('table_glyph_projection', 'visible')
        if projection not in ('visible', 'unclipped'):
            raise ValueError('Unknown table glyph projection')
        visible_fixtures = None
        if projection == 'unclipped':
            visible = json.loads(checked_path(folder, entry['visible_reference']).read_text(encoding='utf-8'))
            if (visible.get('table_glyph_projection') != 'visible' or visible.get('pdf_text_flags') != pymupdf.TEXTFLAGS_RAWDICT
                    or visible['word'] != capture['word'] or visible['capture_id'] != capture_id
                    or visible['word_product_version'] != version or visible['tolerance_pt'] != TOLERANCE_PT):
                raise ValueError('Visible text proof differs from Word capture')
            visible_fixtures = {fixture['name']: fixture for fixture in visible['fixtures']}
            if (set(visible_fixtures) != {fixture['name'] for fixture in reference['fixtures']}
                    or len(visible_fixtures) != len(visible['fixtures'])):
                raise ValueError('Missing or duplicate visible text proof')
        originals = {doc['name']: doc for doc in capture['documents']}
        for fixture in reference['fixtures']:
            name = fixture['name']
            if name not in required:
                continue
            original = originals[name]
            if (fixture['docx_sha256'] != original['sha256']
                    or fixture['pdf_sha256'] != original['pdf_sha256']):
                raise ValueError('Registered document and PDF checksums differ from capture')
            docx = core / 'public/fixtures' / (name + '.docx')
            word = checked_path(folder, original['pdf'])
            verify_hash(docx, original['sha256'])
            verify_hash(word, original['pdf_sha256'])
            key = (entry['word_product_version'], capture['capture_id'], name)
            if key in seen:
                raise ValueError('Duplicate registered border case')
            seen.add(key)
            destination = output / capture['capture_id']
            destination.mkdir(exist_ok=True)
            native = destination / (name + '.pdf')
            command = [str(executable.resolve()), str(docx), str(native.with_suffix('.json')), 'windows', str(native)]
            if projection == 'unclipped':
                command.append('--unclipped-tables')
            subprocess.run(command, check=True)
            if word.read_bytes() == native.read_bytes():
                raise ValueError('Independent native PDF required; refusing a Word self-comparison')
            pages = compare(word, native)
            if len(pages) != original['pages']:
                raise ValueError('Native page count differs from the Word capture')
            result = dict(word_product_version=key[0], capture_id=key[1], name=name,
                          pages=pages, passed=all(p['word_uncovered_count'] ==
                          p['native_uncovered_count'] == 0 for p in pages))
            if visible_fixtures is not None:
                visible_fixture = visible_fixtures[name]
                for field in ('docx_sha256', 'pdf_sha256', 'pdf_file', 'page_count'):
                    if visible_fixture[field] != fixture[field]:
                        raise ValueError('Visible text source differs from drawing reference')
                result['visible_text'] = compare_visible_text(visible_fixture['glyphs'], visible_pdf_glyphs(native))
                result['passed'] &= result['visible_text']['passed']
            results.append(result)
            (output / 'comparison.json').write_text(json.dumps(results, indent=2) + '\n', encoding='utf-8')
            print(name, result['passed'], flush=True)
    if required - {name for _, _, name in seen}:
        raise ValueError('Required Word border cases are missing from the reference index')
    if not all(r['passed'] for r in results):
        raise ValueError(f'Word border comparison failed; differences: {output}')
    print(f'{len(results)} versioned documents passed at {TOLERANCE_PT}pt; {output}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--index', type=Path, required=True)
    parser.add_argument('--native-executable', type=Path, required=True)
    parser.add_argument('--output-directory', type=Path, required=True)
    args = parser.parse_args()
    run(args.index, args.native_executable, args.output_directory)
