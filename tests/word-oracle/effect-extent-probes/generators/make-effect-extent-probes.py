#!/usr/bin/env python3
"""Word probes: the space `wp:effectExtent` reserves for an inline picture.

usage: make-effect-extent-probes.py <out-dir>

inline-shadow-effect-extent.docx
    A 40pt by 30pt inline picture with an outer shadow (`a:outerShdw`, 4pt
    blur, 3pt at 45 degrees) between two lines of text, and its
    `wp:effectExtent` as Word for Windows writes for that shadow. The line
    holding the picture reserves the extent grown by the effect extent; the
    text after it in the line moves right by l + r, and the next line moves
    down by t + b.

inline-rotation-30-effect-extent.docx
    A 40pt by 20pt inline picture rotated 30 degrees, with the effect
    extent Word writes for the rotated bounding box, between two lines.

Both are synthetic and have not yet been opened in Word: capture them as the
other Word oracle fixtures are (Save As > PDF on Mac, drawing origins on
Windows) before adding coordinate references.
"""
import math
import os
import struct
import sys
import zipfile
import zlib

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
EMU_PER_PT = 12700


def png():
    raw = b"".join(b"\x00" + b"\xff\x00\x00" * 4 for _ in range(4))

    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF))

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 4, 4, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def picture(index, cx, cy, effect, rotation_deg=0, shadow=False):
    left, top, right, bottom = effect
    rot = f' rot="{round(rotation_deg * 60000)}"' if rotation_deg else ""
    effects = ('<a:effectLst><a:outerShdw blurRad="50800" dist="38100" dir="2700000" algn="tl" rotWithShape="0">'
               '<a:prstClr val="black"><a:alpha val="40000"/></a:prstClr></a:outerShdw></a:effectLst>'
               if shadow else "")
    return (f'<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0" '
            'xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing">'
            f'<wp:extent cx="{cx}" cy="{cy}"/>'
            f'<wp:effectExtent l="{left}" t="{top}" r="{right}" b="{bottom}"/>'
            f'<wp:docPr id="{index}" name="Picture {index}"/>'
            '<a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">'
            '<a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">'
            '<pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">'
            f'<pic:nvPicPr><pic:cNvPr id="0" name="p.png"/><pic:cNvPicPr/></pic:nvPicPr>'
            '<pic:blipFill><a:blip r:embed="rIdImg" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/>'
            '<a:stretch><a:fillRect/></a:stretch></pic:blipFill>'
            f'<pic:spPr><a:xfrm{rot}><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>'
            f'<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>{effects}</pic:spPr></pic:pic>'
            '</a:graphicData></a:graphic></wp:inline></w:drawing></w:r>')


def text(value):
    return f'<w:r><w:t xml:space="preserve">{value}</w:t></w:r>'


def document(drawing):
    body = (f'<w:p>{text("Before the picture.")}</w:p>'
            f'<w:p>{text("Left ")}{drawing}{text(" right of the picture.")}</w:p>'
            f'<w:p>{text("After the picture.")}</w:p>')
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}'
            '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" '
            'w:left="1440" w:header="720" w:footer="720" w:gutter="0"/><w:docGrid w:linePitch="360"/>'
            '</w:sectPr></w:body></w:document>')


def shadow_probe():
    cx, cy = 40 * EMU_PER_PT, 30 * EMU_PER_PT
    # Word's effect extent for this shadow: the blur on every side, offset
    # by the shadow's distance towards the bottom right.
    return document(picture(1, cx, cy, (12700, 12700, 53975, 53975), shadow=True))


def rotation_probe():
    cx, cy = 40 * EMU_PER_PT, 20 * EMU_PER_PT
    radians = math.radians(30)
    width = cx * math.cos(radians) + cy * math.sin(radians)
    height = cx * math.sin(radians) + cy * math.cos(radians)
    grow_x, grow_y = round((width - cx) / 2), round((height - cy) / 2)
    return document(picture(1, cx, cy, (grow_x, grow_y, grow_x, grow_y), rotation_deg=30))


STYLES = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}"><w:docDefaults>'
          '<w:rPrDefault><w:rPr><w:rFonts w:ascii="Times New Roman" w:hAnsi="Times New Roman" w:cs="Times New Roman"/>'
          '<w:sz w:val="24"/><w:szCs w:val="24"/><w:lang w:val="en-US"/></w:rPr></w:rPrDefault><w:pPrDefault/>'
          '</w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/>'
          '</w:style></w:styles>')
SETTINGS = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:w="{W}">'
            '<w:defaultTabStop w:val="720"/><w:compat><w:compatSetting w:name="compatibilityMode" '
            'w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>')
CONTENT_TYPES = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
    '<Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/>'
    '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
    '<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>'
    '<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>')
RELS = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>')
DOCUMENT_RELS = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
    '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>'
    '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/>'
    '<Relationship Id="rIdImg" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/p.png"/></Relationships>')


def write(path, document_xml):
    # A fixed timestamp keeps the archives byte-for-byte reproducible.
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, data in (("[Content_Types].xml", CONTENT_TYPES), ("_rels/.rels", RELS),
                           ("word/_rels/document.xml.rels", DOCUMENT_RELS),
                           ("word/document.xml", document_xml), ("word/styles.xml", STYLES),
                           ("word/settings.xml", SETTINGS), ("word/media/p.png", png())):
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, data)


def main():
    out = sys.argv[1]
    write(os.path.join(out, "inline-shadow-effect-extent.docx"), shadow_probe())
    write(os.path.join(out, "inline-rotation-30-effect-extent.docx"), rotation_probe())


if __name__ == "__main__":
    main()
