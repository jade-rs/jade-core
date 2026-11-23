"""Bind EMF glyph indexes to independently retained original Word font data.

No drawing coordinates enter this resolver. The caller must verify the raw
archive/source identity before using it. Unbound/ambiguous indexes fail.
"""
import hashlib
import io
import re


def require(value, message):
    if not value:
        raise ValueError(message)


def original_pdf_glyph_index_resolver(pdf_bytes, identity, *, source_sha256,
                                      capture_sha256, word):
    from fontTools.ttLib import TTFont
    import pymupdf
    require(isinstance(identity, dict), 'Invalid glyph-index identity object')
    require(identity.get('schema') == 1 and identity.get('platform') == 'windows',
            'Invalid glyph-index font identity schema/platform')
    require(identity.get('source_sha256') == source_sha256
            and identity.get('capture_sha256') == capture_sha256
            and identity.get('word') == word,
            'Glyph-index source/capture/Word identity changed')
    require(identity.get('original_pdf_sha256') == hashlib.sha256(pdf_bytes).hexdigest(),
            'Glyph-index original PDF identity changed')
    fonts = identity.get('fonts')
    require(isinstance(fonts, list) and 0 < len(fonts) <= 32,
            'Glyph indexes require bounded original font identities')
    programs = {}
    with pymupdf.open(stream=pdf_bytes, filetype='pdf') as pdf:
        for xref in {f[0] for page in pdf for f in page.get_fonts(full=True)}:
            _, _, _, program = pdf.extract_font(xref)
            if program:
                programs[hashlib.sha256(program).hexdigest()] = program
    tables = {}
    for binding in fonts:
        require(isinstance(binding, dict), 'Invalid glyph-index font binding object')
        family, weight, italic = (binding.get(k) for k in ('family', 'weight', 'italic'))
        require(isinstance(family, str) and bool(family) and type(weight) is int
                and 1 <= weight <= 1000 and type(italic) is bool,
                'Invalid glyph-index font face/style identity')
        key = (family.casefold(), weight, italic)
        require(key not in tables, 'Ambiguous glyph-index font face/style')
        program = programs.get(binding.get('original_pdf_font_sha256'))
        require(program is not None, 'Bound original glyph-index font program missing')
        require(isinstance(binding.get('inspected_full_font_sha256'), str)
                and re.fullmatch(r'[0-9a-f]{64}', binding['inspected_full_font_sha256']),
                'Missing full-face inspection identity')
        font = TTFont(io.BytesIO(program))
        try:
            require(all(k in font for k in ('glyf', 'hmtx', 'head', 'maxp', 'OS/2', 'name')),
                    'Glyph-index binding requires an original TrueType outline program')
            families = {n.toUnicode().casefold() for n in font['name'].names if n.nameID in (1, 16)}
            require(family.casefold() in families and font['OS/2'].usWeightClass == weight
                    and bool(font['OS/2'].fsSelection & 1) == italic,
                    'Original glyph-index font face/style differs')
            require(binding.get('font_revision') == font['head'].fontRevision
                    and binding.get('glyph_count') == font['maxp'].numGlyphs,
                    'Glyph-index full-face revision/index domain differs')
            reverse = {}
            for scalar, name in (font.getBestCmap() or {}).items():
                reverse.setdefault(font.getGlyphID(name), set()).add(scalar)
            entries = binding.get('glyphs')
            require(isinstance(entries, list) and bool(entries), 'Empty glyph-index identity table')
            mapping = {}
            for entry in entries:
                require(isinstance(entry, dict), 'Invalid glyph-index scalar binding object')
                gid, scalar = entry.get('glyph_index'), entry.get('unicode_scalar')
                require(type(gid) is int and 0 < gid < font['maxp'].numGlyphs
                        and gid not in mapping and type(scalar) is int
                        and 0 <= scalar <= 0x10ffff and not 0xd800 <= scalar <= 0xdfff,
                        'Invalid or duplicate glyph-index scalar binding')
                require(reverse.get(gid) == {scalar}, 'Ambiguous or changed original font cmap')
                name = font.getGlyphName(gid)
                outline = font['glyf'][name].compile(font['glyf'])
                require(hashlib.sha256(outline).hexdigest() == entry.get('outline_sha256')
                        and list(font['hmtx'][name]) == entry.get('advance_and_bearing'),
                        'Same-index original font outline/advance changed')
                mapping[gid] = chr(scalar)
            tables[key] = mapping
        finally:
            font.close()

    def resolve(selected_font, glyph_index):
        family, weight, italic = selected_font
        mapping = tables.get((family.casefold(), weight, italic))
        require(mapping is not None, 'Selected EMF font face/style is not bound')
        require(glyph_index in mapping, 'Selected EMF glyph index is not bound')
        return mapping[glyph_index]
    return resolve
