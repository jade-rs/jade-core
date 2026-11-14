"""Assert note-rule paint against original Windows Word PDF and EMF geometry."""
import argparse
import hashlib
from pathlib import Path
import struct

from check_platform_coordinates import ROOT, checked_file, read_json, require
from check_windows_font_coordinates import verify as verify_fonts
from compare_windows_borders import rectangles
from extract_windows_emf import emf_glyphs

INDEX = Path('tests/word-oracle/windows/note-rules.json')
INDEX_SHA256 = '4fb8a2fa0172334c6f1c0403df1b131b3bfec27799dfa688536bac5ef9fac863'
TOLERANCE = .375


def emf_rules(data, page, width, height):
    # Reuse the fail-closed header, record, whole-page and text-mode checks.
    emf_glyphs(data, page, width, height)
    state = dict(canvas=None, brush=None, unsupported=False, clip=None)
    brushes, stack, result = {}, [], []
    offset = 0
    while offset < len(data):
        kind, size = struct.unpack_from('<II', data, offset)
        record = data[offset:offset + size]
        if kind == 9:
            state['canvas'] = struct.unpack_from('<ii', record, 8)
        elif kind == 39:
            handle, style, color, _ = struct.unpack_from('<IIII', record, 8)
            brushes[handle] = color if style == 0 else None
        elif kind == 37:
            handle, = struct.unpack_from('<I', record, 8)
            if handle in brushes:
                state['brush'] = brushes[handle]
            elif handle == 0x80000004:  # BLACK_BRUSH
                state['brush'] = 0
            elif handle == 0x80000000:  # WHITE_BRUSH
                state['brush'] = 0xffffff
            elif handle in (0x80000001, 0x80000002, 0x80000003, 0x80000005):
                # Unsupported gray stock brushes and NULL_BRUSH must never
                # inherit an earlier black selection.
                state['brush'] = None
        elif kind == 40:
            brushes.pop(struct.unpack_from('<I', record, 8)[0], None)
        elif kind == 33:
            stack.append(state.copy())
        elif kind == 34:
            relative, = struct.unpack_from('<i', record, 8)
            require(relative < 0 and -relative <= len(stack), 'Unsupported rule RestoreDC')
            state = stack[relative]
            del stack[relative:]
        elif kind in (10, 12):
            state['unsupported'] |= struct.unpack_from('<ii', record, 8) != (0, 0)
        elif kind in (17, 35, 36, 98, 115, 119, 120):
            state['unsupported'] = True
        elif kind == 30:
            clip = struct.unpack_from('<iiii', record, 8)
            previous = state['clip']
            state['clip'] = clip if previous is None else (
                max(clip[0], previous[0]), max(clip[1], previous[1]),
                min(clip[2], previous[2]), min(clip[3], previous[3]))
        elif kind == 76:
            # MS-EMF EMR_BITBLT: logical destination x/y/width/height;
            # PATCOPY paints the selected brush, without a source bitmap.
            # https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-emf/ccd6a1e9-f5f9-452d-b7c7-ed361149b7fa
            require(size == 100 and not state['unsupported'] and state['canvas'] is not None,
                    'Unsupported rule transform or bitmap record')
            x, y, w, h, operation = struct.unpack_from('<iiiiI', record, 24)
            require(operation == 0x00f00021, 'Unsupported rule raster operation')
            require(not any(struct.unpack_from('<IIII', record, 84)), 'Unexpected rule bitmap')
            color = state['brush']
            require(color is not None and color >> 24 in (0, 2), 'Unsupported rule brush')
            if color & 0xffffff == 0xffffff:
                offset += size
                continue
            require(color & 0xffffff == 0, 'Unsupported colored rule')
            require(w > 0 and h > 0, 'Invalid rule destination')
            box = (x, y, x + w, y + h)
            clip = state['clip']
            if clip is not None:
                box = (max(box[0], clip[0]), max(box[1], clip[1]),
                       min(box[2], clip[2]), min(box[3], clip[3]))
            if box[2] > box[0] and box[3] > box[1]:
                sx, sy = width / state['canvas'][0], height / state['canvas'][1]
                result.append([page, box[0] * sx, box[1] * sy, box[2] * sx, box[3] * sy])
        offset += size
    return result


def pdf_rules(path):
    return [[page, *box] for page, boxes in enumerate(rectangles(path), 1) for box in boxes]


def compare_rules(expected, actual, label):
    require(len(actual) == len(expected) and bool(expected), f'{label}: missing/extra rule paint')
    expected, actual = sorted(expected), sorted(actual)
    for word, native in zip(expected, actual):
        require(word[0] == native[0], f'{label}: rule page changed')
        for axis in range(1, 5):
            require(abs(word[axis] - native[axis]) <= TOLERANCE,
                    f'{label}: rule axis {axis}: Word {word}, native {native}')
    return len(expected)


def verify(root=ROOT):
    index_path = root / INDEX
    require(hashlib.sha256(index_path.read_bytes()).hexdigest() == INDEX_SHA256,
            'Note-rule coverage/expectations changed')
    index = read_json(index_path)
    require(index['schema'] == 1 and len(index['fixtures']) == 22, 'Note-rule coverage removed')
    fonts = {row[0]['name']: row for row in verify_fonts(root)}
    cases, names = [], set()
    checked_file(root, index['font_tables'])
    for entry in index['fixtures']:
        name = entry['name']
        require(name not in names and name in fonts, 'Duplicate or unregistered note-rule input')
        names.add(name)
        case, source, manifest, document, _ = fonts[name]
        require(case['word_product_version'] == entry['word_product_version'], 'Wrong rule Word version')
        pdf = manifest.parent / document['pdf']
        word = pdf_rules(pdf)
        require(len(word) == entry['rule_count'] == 1, 'Original rule count changed')
        emf = []
        for page in document['metafiles']:
            emf += emf_rules((manifest.parent / page['file']).read_bytes(), page['page'],
                             page['width_pt'], page['height_pt'])
        compare_rules(word, emf, name + '/original-EMF')
        cases.append((case, source, manifest, document, word, emf))
    return cases


def check(native_directory, root=ROOT):
    for case, _, manifest, _, word, emf in verify(root):
        target = native_directory / manifest.parent.name / case['name']
        actual = read_json(target / 'native.json')
        require(actual['input_sha256'] == case['source']['sha256'], 'Wrong rule rendered input')
        native = pdf_rules(target / 'native.pdf')
        compare_rules(word, native, case['name'] + '/PDF-rule')
        compare_rules(emf, native, case['name'] + '/EMF-rule')
        print(case['name'], case['word_product_version'], 'PDF/EMF rule paint: PASS', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-directory', type=Path)
    args = parser.parse_args()
    if args.native_directory:
        check(args.native_directory.resolve())
    else:
        print(f'Verified {len(verify())} original Windows Word note rules')
