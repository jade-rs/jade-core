"""Keep all painted glyphs; canonicalize U+00AD only with font-ink proof."""
from io import BytesIO
import hashlib
import pymupdf
from fontTools.ttLib import TTFont
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.recordingPen import DecomposingRecordingPen


def painted_origins(path):
    glyphs=[];aliases=[]
    with pymupdf.open(path) as document:
        for page_index,page in enumerate(document,1):
            fonts=None
            for span in page.get_texttrace():
                if span['type']==3 or span.get('opacity',1)<=0:
                    continue
                for char,gid,origin,box in span['chars']:
                    character=chr(char)
                    if character.isspace():
                        continue
                    if char==173:
                        if fonts is None:
                            fonts=[]
                            for record in page.get_fonts(full=True):
                                label,extension,_,data=document.extract_font(record[0])
                                if data and extension in ('ttf','otf','ttc'):
                                    fonts.append((label,record[3],TTFont(BytesIO(data)),data))
                        candidates=[f for f in fonts if span['font'].split('+')[-1]
                                    in (f[0].split('+')[-1],f[1].split('+')[-1])]
                        if not candidates:
                            raise ValueError('Cannot identify the actual font of painted U+00AD')
                        verified=[];geometries=[]
                        for label,_,font,data in candidates:
                            if gid>=len(font.getGlyphOrder()):
                                continue
                            name=font.getGlyphName(gid);hyphen=font.getBestCmap().get(45)
                            glyph_set=font.getGlyphSet()
                            actual=DecomposingRecordingPen(glyph_set);glyph_set[name].draw(actual)
                            expected=DecomposingRecordingPen(glyph_set)
                            if hyphen is None:
                                raise ValueError('Painted control font has no canonical hyphen')
                            glyph_set[hyphen].draw(expected)
                            bounds=BoundsPen(glyph_set);glyph_set[name].draw(bounds)
                            if not bounds.bounds or not (bounds.bounds[0]<bounds.bounds[2] and bounds.bounds[1]<bounds.bounds[3]):
                                raise ValueError('Painted U+00AD has no verified visible ink')
                            if actual.value!=expected.value or glyph_set[name].width!=glyph_set[hyphen].width:
                                raise ValueError('Painted U+00AD is not the actual font hyphen glyph')
                            geometry=(font['head'].unitsPerEm,glyph_set[name].width,actual.value)
                            geometries.append(geometry)
                            verified.append(dict(font=label,program_sha256=hashlib.sha256(data).hexdigest(),
                                glyph_id=gid,glyph_name=name,hyphen_name=hyphen,ink_bounds=list(bounds.bounds),
                                geometry_sha256=hashlib.sha256(repr(geometry).encode()).hexdigest()))
                        if not verified or any(g!=geometries[0] for g in geometries):
                            raise ValueError('Ambiguous painted control font has different glyph geometry')
                        aliases.append(dict(page=page_index,source_char=character,canonical_char='-',
                                            origin=list(origin),equivalent_font_candidates=verified))
                        character='-'
                    glyphs.append([page_index,character,*origin])
        return dict(page_count=len(document),glyphs=glyphs,verified_painted_aliases=aliases)
