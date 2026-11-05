"""Extract absolute Word drawing origins from Page.EnhMetaFileBits.

MS-EMF EMR_EXTTEXTOUTW supplies page-unit coordinates and character advances.
The whole-page logical canvas maps to Word's exact PageSetup dimensions.
This avoids printer drivers' rounded millimetre sizes (e.g. 216x279mm for
US Letter). No glyph alignment or fitted scale/offset is used.
Unsupported text modes fail instead of being guessed.

https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-emf/a59a79ac-328e-492d-a34d-e02727af6edf
https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-emf/dd585d0a-5d7c-4034-963a-1141af836972
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import struct
import unicodedata


def emf_glyphs(data, page, width_pt, height_pt):
    if len(data) < 88 or len(data) > 64 * 1024 * 1024:
        raise ValueError('Invalid EMF size')
    if struct.unpack_from('<II', data)[0] != 1 or struct.unpack_from('<I', data, 40)[0] != 0x464d4520:
        raise ValueError('Invalid EMF header')
    if struct.unpack_from('<I', data, 48)[0] != len(data):
        raise ValueError('Truncated or appended EMF')
    expected_records = struct.unpack_from('<I', data, 52)[0]
    left, top, right, bottom = struct.unpack_from('<iiii', data, 24)
    if left != 0 or top != 0 or right <= 0 or bottom <= 0:
        raise ValueError('EMF must describe a whole-page canvas at zero origin')
    # Text alignment and transformations are saved/restored with the DC.
    state = dict(alignment=0, unsupported_transform=False, canvas=None, rotated_font=False)
    canvas = None
    fonts = {}
    if not all(math.isfinite(v) and v > 0 for v in (width_pt, height_pt)):
        raise ValueError('Invalid Word paper dimensions')
    if abs(width_pt * 25.4 / 0.72 - right) > 100 or abs(height_pt * 25.4 / 0.72 - bottom) > 100:
        raise ValueError('Word paper does not match the recorded EMF page')
    stack, glyphs = [], []
    offset, records, eof = 0, 0, False
    while offset < len(data):
        if offset + 8 > len(data):
            raise ValueError('Truncated EMF record')
        kind, size = struct.unpack_from('<II', data, offset)
        if size < 8 or size % 4 or offset + size > len(data) or eof:
            raise ValueError('Invalid EMF record bounds')
        record = data[offset:offset + size]
        if kind == 9:
            if size != 16:
                raise ValueError('Invalid logical canvas')
            extent = struct.unpack_from('<ii', record, 8)
            if min(extent) <= 0 or (canvas is not None and extent != canvas):
                raise ValueError('Unsupported nested logical canvas')
            canvas = state['canvas'] = extent
        elif kind == 22:
            if size != 12:
                raise ValueError('Invalid text alignment record')
            state['alignment'] = struct.unpack_from('<I', record, 8)[0]
        elif kind == 82:
            if size < 104:
                raise ValueError('Truncated font record')
            handle = struct.unpack_from('<I', record, 8)[0]
            angles = struct.unpack_from('<ii', record, 20)
            family = record[40:104].decode('utf-16-le').split('\0')[0]
            fonts[handle] = angles != (0, 0) or family.startswith('@')
        elif kind == 37:
            if size != 12:
                raise ValueError('Invalid object selection')
            handle = struct.unpack_from('<I', record, 8)[0]
            if handle in fonts:
                state['rotated_font'] = fonts[handle]
            elif handle in (0x8000000a, 0x8000000b, 0x8000000c, 0x8000000d, 0x80000010, 0x80000011):
                state['rotated_font'] = False
        elif kind == 40:
            if size != 12:
                raise ValueError('Invalid object deletion')
            fonts.pop(struct.unpack_from('<I', record, 8)[0], None)
        elif kind == 33:
            stack.append(state.copy())
        elif kind == 34:
            if size != 12:
                raise ValueError('Invalid RestoreDC')
            relative = struct.unpack_from('<i', record, 8)[0]
            if relative >= 0 or -relative > len(stack):
                raise ValueError('Unsupported RestoreDC index')
            state = stack[relative]
            del stack[relative:]
        elif kind in (10, 12):
            if size != 16:
                raise ValueError('Invalid mapping origin')
            if struct.unpack_from('<ii', record, 8) != (0, 0):
                state['unsupported_transform'] = True
        elif kind in (17, 35, 36, 98, 115, 119, 120):
            # These may alter text placement beyond the per-record scale.
            # Preserve the raw file; require explicit support before promotion.
            state['unsupported_transform'] = True
        elif kind in (83, 96, 97, 108):
            raise ValueError(f'Unsupported text drawing record {kind}')
        elif kind == 84:
            if size < 76:
                raise ValueError('Truncated Unicode text record')
            mode = struct.unpack_from('<I', record, 24)[0]
            scale_x, scale_y = struct.unpack_from('<ff', record, 28)
            x, y, count, text_at, flags = struct.unpack_from('<iiIII', record, 36)
            dx_at = struct.unpack_from('<I', record, 72)[0]
            if mode != 1 or state['alignment'] != 24 or state['unsupported_transform'] or state['rotated_font'] or state['canvas'] is None:
                raise ValueError('Unsupported graphics mode, alignment or transform')
            if not all(math.isfinite(v) and v > 0 for v in (scale_x, scale_y)):
                raise ValueError('Invalid drawing-unit scale')
            if abs(scale_x * canvas[0] - right) > 2 or abs(scale_y * canvas[1] - bottom) > 2:
                raise ValueError('Text scale does not describe the whole-page canvas')
            if flags & ~0x2006:
                raise ValueError(f'Unsupported text options {flags:#x}')
            stride = 2 if flags & 0x2000 else 1
            if count > 1_000_000 or text_at < 76 or text_at + count * 2 > size:
                raise ValueError('Invalid UTF-16 string bounds')
            if dx_at < 76 or dx_at + count * stride * 4 > size:
                raise ValueError('Missing or truncated character advances')
            text = record[text_at:text_at + count * 2].decode('utf-16-le')
            if len(text) != count:
                raise ValueError('Surrogate-pair advances need explicit support')
            advances = struct.unpack_from('<' + 'i' * (count * stride), record, dx_at)
            # Word paints the recorded whole-page canvas onto its own paper.
            # EMF printer millimetres can be rounded; PageSetup points retain
            # twip precision. This conversion uses no native glyph positions.
            pt_x, pt_y = width_pt / canvas[0], height_pt / canvas[1]
            for index, char in enumerate(text):
                if not char.isspace() and unicodedata.category(char) != 'Cc':
                    glyphs.append([page, char, x * pt_x, y * pt_y])
                x += advances[index * stride]
                if stride == 2:
                    y += advances[index * stride + 1]
        elif kind == 14:
            eof = True
        offset += size
        records += 1
    if not eof or records != expected_records or stack:
        raise ValueError('Incomplete EMF record/DC stream')
    return glyphs


def extract(manifest_path):
    manifest_path = Path(manifest_path)
    capture = json.loads(manifest_path.read_text(encoding='utf-8-sig'))
    if capture.get('schema') != 2 or capture.get('status') != 'complete' or capture.get('platform') != 'windows':
        raise ValueError('Only complete versioned Word captures can become evidence')
    version = capture['word']['product_version']
    if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+', version):
        raise ValueError('Complete Word product version required')
    fixtures = []
    for item in capture['documents']:
        pdf_name = item['pdf']
        if '/' in pdf_name or '\\' in pdf_name or pdf_name in ('', '.', '..'):
            raise ValueError('PDF must belong to the capture directory')
        if hashlib.sha256((manifest_path.parent / pdf_name).read_bytes()).hexdigest() != item['pdf_sha256']:
            raise ValueError('Companion Word PDF changed')
        pages = item.get('metafiles', [])
        if [p['page'] for p in pages] != list(range(1, item['pages'] + 1)):
            raise ValueError('Missing, duplicate or reordered Word metafile pages')
        glyphs = []
        for page in pages:
            name = page['file']
            if '/' in name or '\\' in name or name in ('', '.', '..'):
                raise ValueError('Metafile must belong to the capture directory')
            data = (manifest_path.parent / name).read_bytes()
            if hashlib.sha256(data).hexdigest() != page['sha256']:
                raise ValueError(f'Word metafile changed: {name}')
            glyphs.extend(emf_glyphs(data, page['page'], page['width_pt'], page['height_pt']))
        fixtures.append(dict(name=item['name'], docx_sha256=item['sha256'],
                             pdf_sha256=item['pdf_sha256'], pdf_file=item['pdf'],
                             metafiles=pages, page_count=item['pages'], glyphs=glyphs))
    return dict(schema=2, platform='windows', word_product_version=version,
                word=capture['word'], capture_id=capture['capture_id'],
                captured_at_utc=capture['captured_at_utc'],
                coordinate_source='Word Page.EnhMetaFileBits / MS-EMF',
                extractor='MS-EMF GM_COMPATIBLE baseline Unicode origins v1',
                tolerance_pt=0.375, fixtures=fixtures)


def verify_index(index_path):
    index_path = Path(index_path)
    index = json.loads(index_path.read_text(encoding='utf-8'))
    checked = 0
    for entry in index['references']:
        directory = index_path.parent / entry['directory']
        reference = json.loads((directory / entry['reference']).read_text(encoding='utf-8'))
        if reference.get('coordinate_source') != 'Word Page.EnhMetaFileBits / MS-EMF':
            continue
        full = extract(directory / 'capture.json')
        if full != json.loads((directory / 'word-emf-reference.json').read_text(encoding='utf-8')):
            raise ValueError('Archived drawing origins differ from the raw Word EMFs')
        names = {f['name'] for f in reference['fixtures']}
        selected = dict(full, fixtures=[f for f in full['fixtures'] if f['name'] in names])
        if selected != reference or len(names) != entry['fixture_count']:
            raise ValueError('Strict drawing origins differ from the raw Word EMFs')
        checked += sum(len(f['glyphs']) for f in reference['fixtures'])
    return checked


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('manifest', type=Path, nargs='?')
    parser.add_argument('--verify', type=Path)
    parser.add_argument('--verify-index', type=Path)
    args = parser.parse_args()
    if args.verify_index:
        print(f'Verified {verify_index(args.verify_index)} raw Word drawing origins')
        raise SystemExit(0)
    if args.manifest is None:
        parser.error('manifest or --verify-index is required')
    reference = extract(args.manifest)
    if args.verify:
        if reference != json.loads(args.verify.read_text(encoding='utf-8')):
            raise ValueError('Archived drawing origins differ from the raw Word EMFs')
    else:
        target = args.manifest.parent / 'word-emf-reference.json'
        if target.exists():
            raise FileExistsError(f'Refusing to overwrite evidence: {target}')
        target.write_bytes((json.dumps(reference, ensure_ascii=False, indent=1) + '\n').encode('utf-8'))
        print(target)
