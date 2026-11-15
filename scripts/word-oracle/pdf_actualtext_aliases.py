"""Honor scalar PDF ActualText only for proven aliases of the same font glyph.

Paint traces expose a font's canonical cmap character, while searchable text
honors ActualText. Never normalize Unicode or infer labels from expectations:
require explicit metadata for every painted copy and identical nonzero glyph
IDs in the embedded font program. Coordinates and paint counts stay unchanged.
"""
from collections import Counter, defaultdict
import re
import struct
import pymupdf
from pypdf import PdfReader
from pypdf.generic import ContentStream

from check_platform_coordinates import require


def base_name(value):
    return re.sub(r'^[A-Z]{6}\+', '', str(value).lstrip('/'))


def text_bytes(value):
    if hasattr(value, 'original_bytes'):
        return value.original_bytes
    return bytes(value)


def postscript_name(program):
    """Resolve PDF display names through the embedded OpenType name ID 6.

    Name table layout: https://learn.microsoft.com/typography/opentype/spec/name
    This binds font identifiers; it never normalizes character Unicode.
    """
    if len(program) < 12 or program[:4] not in (b'\x00\x01\x00\x00', b'OTTO', b'true', b'typ1'):
        return None
    tables, = struct.unpack_from('>H', program, 4)
    require(12 + 16 * tables <= len(program), 'Truncated embedded font directory')
    for index in range(tables):
        record = 12 + 16 * index
        if program[record:record+4] != b'name':
            continue
        offset, length = struct.unpack_from('>II', program, record + 8)
        require(offset + length <= len(program) and length >= 6, 'Truncated embedded font names')
        names = program[offset:offset+length]
        version, count, storage = struct.unpack_from('>HHH', names)
        require(version in (0, 1) and 6 + 12 * count <= storage <= len(names), 'Invalid embedded font names')
        found = set()
        for number in range(count):
            platform, encoding, language, name_id, size, start = struct.unpack_from('>HHHHHH', names, 6 + 12 * number)
            if name_id != 6 or platform not in (0, 1, 3):
                continue
            require(storage + start + size <= len(names), 'Truncated PostScript font name')
            data = names[storage+start:storage+start+size]
            value = data.decode('utf-16-be' if platform in (0, 3) else 'mac_roman')
            found.add(base_name(value))
        require(len(found) <= 1, 'Ambiguous embedded PostScript font name')
        return next(iter(found), None)
    return None


def actual_glyphs(reader, page):
    try:
        fonts = page['/Resources']['/Font']
    except KeyError:
        fonts = {}
    active, saved, groups = None, [], []
    records = defaultdict(Counter)
    for operands, operator in ContentStream(page.get_contents(), reader).operations:
        if operator == b'q':
            saved.append(active)
        elif operator == b'Q':
            active = saved.pop()
        elif operator == b'Tf':
            active = operands[0]
        elif operator == b'BDC':
            properties = operands[1]
            if not isinstance(properties, dict):
                properties = page['/Resources']['/Properties'][properties].get_object()
            actual = properties.get('/ActualText')
            groups.append(None if actual is None else dict(actual=str(actual), glyphs=[]))
        elif operator == b'BMC':
            groups.append(None)
        elif operator in (b'Tj', b'TJ', b"'", b'"'):
            values = operands[0] if operator == b'TJ' else [operands[-1]]
            for value in values:
                if not isinstance(value, (str, bytes)):
                    continue
                for group in groups:
                    if group is not None:
                        group['glyphs'].append((active, text_bytes(value)))
        elif operator == b'EMC':
            group = groups.pop()
            if group is None or len(group['actual']) != 1 or len(group['glyphs']) != 1:
                continue
            resource, data = group['glyphs'][0]
            font = fonts[resource].get_object()
            if font.get('/Subtype') != '/Type0' or font.get('/Encoding') != '/Identity-H' or len(data) != 2:
                continue
            descendant = font['/DescendantFonts'][0].get_object()
            if descendant.get('/Subtype') != '/CIDFontType2':
                continue
            cid = int.from_bytes(data, 'big')
            mapping = descendant.get('/CIDToGIDMap', '/Identity')
            if mapping == '/Identity':
                glyph = cid
            else:
                mapping = mapping.get_object().get_data()
                require(len(mapping) >= 2 * (cid + 1), 'Truncated CID-to-glyph table')
                glyph = int.from_bytes(mapping[2*cid:2*cid+2], 'big')
            records[base_name(font['/BaseFont']), glyph][ord(group['actual'])] += 1
    require(not groups and not saved, 'Unbalanced PDF graphics/marked content')
    return records


def resolve_actualtext_aliases(path, all_glyphs):
    reader = PdfReader(path)
    glyph_index = 0
    changed = 0
    with pymupdf.open(path) as document:
        for index, page in enumerate(document):
            explicit = actual_glyphs(reader, reader.pages[index])
            if not explicit:
                glyph_index += sum(1 for span in page.get_texttrace() for ch in span['chars']
                                   if chr(ch[0]).isprintable() and not chr(ch[0]).isspace())
                continue
            traces = [(span, ch) for span in page.get_texttrace() for ch in span['chars']
                      if chr(ch[0]).isprintable() and not chr(ch[0]).isspace()]
            embedded = defaultdict(set)
            aliases = defaultdict(set)
            for ref in page.get_fonts(full=True):
                canonical = postscript_name(document.extract_font(ref[0])[3]) or base_name(ref[3])
                aliases[base_name(ref[3])].add(canonical)
                aliases[canonical].add(canonical)
                embedded[canonical].add(ref[0])

            def font_name(name):
                name = base_name(name)
                choices = aliases.get(name, {name})
                require(len(choices) == 1, 'Ambiguous PDF font identifier')
                return next(iter(choices))

            canonical_explicit = defaultdict(Counter)
            for (name, gid), candidates in explicit.items():
                canonical_explicit[font_name(name), gid].update(candidates)
            explicit = canonical_explicit
            painted = Counter((font_name(span['font']), ch[1]) for span, ch in traces)
            replacements = {}
            for key, candidates in explicit.items():
                # Whitespace/nonprinting glyphs are outside the physical-ink
                # registry. Their omission cannot relabel a printable glyph.
                if painted[key] == 0:
                    continue
                if len(candidates) != 1:
                    raise ValueError('Ambiguous ActualText for one font glyph')
                actual, count = next(iter(candidates.items()))
                canonical = {ch[0] for span, ch in traces if (font_name(span['font']), ch[1]) == key}
                if canonical == {actual}:
                    continue
                if count != painted[key] or len(canonical) != 1 or not embedded[key[0]]:
                    raise ValueError(f'Cannot safely disambiguate ActualText aliases: {key=} {count=} painted={painted[key]} {canonical=} xrefs={embedded[key[0]]}')
                programs = {document.extract_font(xref)[3] for xref in embedded[key[0]]}
                if len(programs) != 1 or not next(iter(programs)):
                    raise ValueError('ActualText alias lacks one unambiguous embedded font program')
                program, = programs
                font = pymupdf.Font(fontbuffer=program)
                source, = canonical
                if not (font.has_glyph(source, fallback=False)
                        == font.has_glyph(actual, fallback=False) == key[1] and key[1] != 0):
                    raise ValueError('ActualText changes the painted glyph, rather than its Unicode alias')
                replacements[key] = actual
            for span, ch in traces:
                glyph = all_glyphs[glyph_index]
                require(glyph[:2] == [index + 1, chr(ch[0])] and tuple(glyph[2:]) == tuple(ch[2]),
                        'PDF paint trace changed during semantic resolution')
                key = font_name(span['font']), ch[1]
                if key in replacements:
                    glyph[1] = chr(replacements[key])
                    changed += 1
                glyph_index += 1
    require(glyph_index == len(all_glyphs), 'PDF paint coverage changed during semantic resolution')
    return changed
