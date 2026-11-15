"""Assert legacy Word text paint, including copies and scalable glyph contours.

PDF rawdict text removes nearby duplicate glyphs and includes native searchable
outline overlays. Paint traces and contour geometry retain the actual drawing
contract without changing the existing four-plane typography gate.
"""
import argparse
from collections import defaultdict
import hashlib
import json
import math
from pathlib import Path
import subprocess
import sys
from zipfile import ZipFile

import numpy as np
import pymupdf

from check_platform_coordinates import ROOT, checked_file, compare, extract_fonts, read_json, require
from extract_windows_emf import extract
from pdf_actualtext_aliases import resolve_actualtext_aliases

INDEX = Path('tests/word-oracle/windows/graphic-text.json')
INDEX_SHA256 = '0ddc6587e48ba90016fb21001cc56176c76620cc7f9c2707745214508c158508'
TOLERANCE_PT = .375


def word_font_sizes(path):
    with pymupdf.open(path) as document:
        return sorted({round(span['size'], 5)
                       for page in document for span in page.get_texttrace()
                       if span['type'] in (0, 1) and any(
                           chr(ch[0]).isprintable() and not chr(ch[0]).isspace()
                           for ch in span['chars'])})


def pdf_paint(path):
    all_glyphs, visible, colors, paths = [], [], defaultdict(list), []
    images = 0
    with pymupdf.open(path) as document:
        for page_index, page in enumerate(document):
            images += len(page.get_image_info())
            for span in page.get_texttrace():
                require(span['type'] in (0, 1, 3), 'Unknown PDF text rendering mode')
                rgb = tuple(round(value * 255) for value in span['color'])
                if len(rgb) == 1:
                    rgb *= 3
                require(len(rgb) == 3, 'Unsupported PDF paint color space')
                key = (*rgb, round(span['opacity'] * 255))
                for unicode, _, origin, _ in span['chars']:
                    char = chr(unicode)
                    if not char.isprintable() or char.isspace():
                        continue
                    glyph = [page_index + 1, char, *origin]
                    all_glyphs.append(glyph)
                    if span['type'] != 3:
                        visible.append(glyph)
                        colors[key].append(glyph)
            leading_white = 0
            for drawing in page.get_drawings():
                # seqno includes text/images: later white paint may erase ink.
                if (drawing['seqno'] == leading_white and drawing['type'] == 'f'
                        and drawing['fill'] == (1., 1., 1.) and drawing['fill_opacity'] == 1):
                    leading_white += 1
                    continue
                paths.append(dict(drawing, page=page_index + 1))
        resolve_actualtext_aliases(path, all_glyphs)
        return dict(page_count=len(document), glyphs=all_glyphs), dict(
            page_count=len(document), glyphs=visible), dict(colors), paths, images


def _point(point):
    value = np.asarray(tuple(point), dtype=float)
    require(value.shape == (2,) and np.isfinite(value).all(), 'Nonfinite outline coordinate')
    return value


def outline_segments(path):
    """Flatten actual PDF commands; no alignment, fitting, scaling or rebasing."""
    segments, subpath_start, previous = [], None, None

    def line(a, b):
        nonlocal subpath_start, previous
        a, b = _point(a), _point(b)
        if previous is None or np.linalg.norm(a - previous) > .0001:
            subpath_start = a
        if np.linalg.norm(b - a) > 1e-9:
            segments.append((a, b))
        previous = b

    for item in path['items']:
        if item[0] == 'l':
            line(item[1], item[2])
        elif item[0] == 'c':
            control = [_point(value) for value in item[1:]]
            require(len(control) == 4, 'Invalid outline cubic')
            previous_point = control[0]
            for index in range(1, 65):
                t = index / 64
                point = ((1-t)**3 * control[0] + 3*(1-t)**2*t * control[1]
                         + 3*(1-t)*t*t * control[2] + t**3 * control[3])
                line(previous_point, point)
                previous_point = point
        elif item[0] == 're':
            rect = item[1]
            points = [rect.tl, rect.tr, rect.br, rect.bl, rect.tl]
            for a, b in zip(points, points[1:]):
                line(a, b)
        elif item[0] == 'qu':
            quad = item[1]
            points = [quad.ul, quad.ur, quad.lr, quad.ll, quad.ul]
            for a, b in zip(points, points[1:]):
                line(a, b)
        else:
            raise ValueError('Unsupported outline command: ' + str(item[0]))
    if path.get('closePath') and previous is not None:
        line(previous, subpath_start)
    require(bool(segments), 'Empty outline path')
    return np.asarray(segments)


def _covered_by(source, target):
    # Sample long straight edges too, so deleting an edge while keeping its
    # endpoints cannot pass. Cubic subdivision has a much finer resolution
    # than the unchanged 0.375 pt absolute displacement budget.
    points = []
    for a, b in source:
        count = max(1, math.ceil(float(np.linalg.norm(b-a)) / .1))
        points.extend(a + (b-a) * (index / count) for index in range(count + 1))
    start = target[:, 0]
    delta = target[:, 1] - start
    length2 = (delta * delta).sum(axis=1)
    for offset in range(0, len(points), 128):
        batch = np.asarray(points[offset:offset + 128])[:, None, :]
        relative = batch - start[None, :, :]
        fraction = np.clip((relative * delta).sum(axis=2) / length2, 0, 1)
        nearest = start[None, :, :] + fraction[:, :, None] * delta
        distance2 = ((batch - nearest)**2).sum(axis=2).min(axis=1)
        require(bool(np.all(distance2 <= TOLERANCE_PT**2)), 'Outline geometry exceeds 0.375 pt')


def compare_outlines(expected, actual):
    require(len(expected) == len(actual), 'Outline path count changed')
    for word, native in zip(expected, actual):
        require(word['page'] == native['page'], 'Outline page changed')
        require(word['type'] == native['type'] == 's', 'Outline paint kind changed')
        require(word['color'] == native['color'], 'Outline color changed')
        require(word['stroke_opacity'] == native['stroke_opacity'], 'Outline opacity changed')
        require(abs(word['width'] - native['width']) <= .01, 'Outline width changed')
        require(word['lineCap'] == native['lineCap'] and word['lineJoin'] == native['lineJoin'],
                'Outline cap/join changed')
        require(''.join(word['dashes'].split()) == ''.join(native['dashes'].split()),
                'Outline dash changed')
        a, b = outline_segments(word), outline_segments(native)
        _covered_by(a, b)
        _covered_by(b, a)


def compare_paint(word_emf, word_pdf, native_pdf, label):
    native_all, native_visible, native_colors, native_paths, native_images = pdf_paint(native_pdf)
    _, word_visible, word_colors, word_paths, word_images = pdf_paint(word_pdf)
    compare(word_emf, native_all, label + '/EMF physical glyph copies')
    compare(word_visible, native_visible, label + '/visible PDF paint')
    require(word_colors.keys() == native_colors.keys(), label + ': visible colors changed')
    for key in word_colors:
        compare(dict(page_count=word_visible['page_count'], glyphs=word_colors[key]),
                dict(page_count=native_visible['page_count'], glyphs=native_colors[key]),
                label + '/RGBA ' + str(key))
    require(native_images == word_images, label + ': raster paint count changed')
    compare_outlines(word_paths, native_paths)


def _font_control(source, control):
    import re
    with ZipFile(source) as a, ZipFile(control) as b:
        require(set(a.namelist()) == set(b.namelist()), 'Font control parts changed')
        for name in a.namelist():
            original, plain = a.read(name), b.read(name)
            if name == 'word/document.xml':
                original, count = re.subn(rb'<w:(shadow|outline|emboss|imprint)\s*/>', b'', original)
                require(count == 4, 'Font control does not cover the original effects')
            require(original == plain, 'Font control changed font/text/settings: ' + name)


def verify(root=ROOT):
    require(hashlib.sha256((root / INDEX).read_bytes()).hexdigest() == INDEX_SHA256,
            'Graphic-text coverage/expectations changed')
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and len(index['fixtures']) == 62, 'Graphic-text coverage removed')
    cases, names, drawings = [], set(), {}
    for case in index['fixtures']:
        name = case['name']
        require(name not in names, 'Duplicate graphic-text case')
        names.add(name)
        source = checked_file(root, case['source'])
        manifest = checked_file(root, case['capture'])
        capture = read_json(manifest)
        require(capture['platform'] == 'windows' and capture['status'] == 'complete', 'Invalid Word capture')
        require(capture['word']['product_version'] == case['word_product_version'], 'Wrong Word version')
        require(manifest.parent.parent.name == 'word-' + case['word_product_version'], 'Wrong version directory')
        document, = [d for d in capture['documents'] if d['name'] == name]
        require(document['sha256'] == case['source']['sha256'], 'Wrong captured input')
        require(document['open_mode'] == 'normal-read-only', 'Wrong Word opening mode')
        require((manifest.parent / 'inputs' / (name + '.docx')).read_bytes() == source.read_bytes(), 'Archived source changed')
        reference = checked_file(root, case['drawing_reference'])
        require(reference.parent == manifest.parent, 'Wrong EMF reference')
        if manifest not in drawings:
            drawings[manifest] = extract(manifest)
            require(read_json(reference) == drawings[manifest], 'Original EMF origins changed')
        drawing, = [row for row in drawings[manifest]['fixtures'] if row['name'] == name]
        require(len(drawing['glyphs']) == case['emf_glyph_count'], 'Physical glyph coverage changed')
        word_pdf = manifest.parent / document['pdf']
        if 'word_font_sizes_pt' in case:
            import xml.etree.ElementTree as ET
            with ZipFile(source) as package:
                body = ET.fromstring(package.read('word/document.xml'))
            w = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
            requested = {float(size.get(w + 'val')) for size in body.findall('.//' + w + 'rPr/' + w + 'sz')}
            require(requested == {2 * case['requested_font_size_pt']}, 'Authored font size changed')
            require(word_font_sizes(word_pdf) == case['word_font_sizes_pt'], 'Original Word draw sizes changed')
        _, visible, _, paths, images = pdf_paint(word_pdf)
        require(len(visible['glyphs']) == case['pdf_visible_glyph_count'], 'Visible PDF coverage changed')
        require(len(paths) == case['outline_count'] and images == case['image_count'], 'Original graphics coverage changed')
        font_pdf = word_pdf
        if case.get('font_control_capture'):
            font_manifest = checked_file(root, case['font_control_capture'])
            font_capture = read_json(font_manifest)
            require(font_capture['status'] == 'complete' and font_capture['word'] == capture['word'],
                    'Font-control Word identity changed')
            font_document, = font_capture['documents']
            control = font_manifest.parent / 'inputs' / (font_document['name'] + '.docx')
            require(hashlib.sha256(control.read_bytes()).hexdigest() == font_document['sha256'], 'Font control input changed')
            _font_control(source, control)
            font_pdf = font_manifest.parent / font_document['pdf']
            require(hashlib.sha256(font_pdf.read_bytes()).hexdigest() == font_document['pdf_sha256'], 'Font-control PDF changed')
        cases.append((case, source, manifest, drawing, word_pdf, font_pdf))
    return cases


def check(executable, output, root=ROOT):
    require(sys.platform == 'win32', 'Graphic text checks require Windows')
    for case, source, manifest, drawing, word_pdf, font_pdf in verify(root):
        target = output / case['name']
        target.mkdir(parents=True, exist_ok=False)
        extract_fonts(font_pdf, target / 'fonts')
        native = target / 'native.pdf'
        snapshot = target / 'native.json'
        subprocess.run([str(executable), str(source), str(snapshot), 'windows', str(native),
                        '--fonts-directory', str(target / 'fonts')], check=True)
        require(read_json(snapshot)['input_sha256'] == case['source']['sha256'], 'Wrong rendered source')
        require(native.read_bytes() != word_pdf.read_bytes(), 'Word self-comparison')
        compare_paint(drawing, word_pdf, native, case['name'])
        print(case['name'], case['word_product_version'], 'physical paint/visible colors/outline geometry: PASS', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/windows-graphic-text')
    args = parser.parse_args()
    if args.native_executable:
        check(args.native_executable.resolve(), args.output_directory.resolve())
    else:
        print(f'Verified {len(verify())} original Word graphics cases')
