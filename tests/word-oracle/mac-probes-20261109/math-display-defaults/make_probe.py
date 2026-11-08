"""Generate deterministic, isolated Word probes; no Office automation is run."""
import hashlib
import json
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).resolve().parent
W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'
M = 'http://schemas.openxmlformats.org/officeDocument/2006/math'
REL = 'http://schemas.openxmlformats.org/package/2006/relationships'
OFFICE = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
CASES = [
    ('absent', '', '', ''),
    ('margins', '<m:lMargin m:val="1440"/><m:rMargin m:val="720"/>', '', ''),
    ('present-without-value', '<m:lMargin/><m:rMargin/>', '', ''),
    ('oversized-left', '<m:lMargin m:val="9000"/><m:rMargin m:val="1440"/>', '', ''),
    ('disabled-left', '<m:dispDef m:val="0"/><m:lMargin m:val="1440"/><m:rMargin m:val="720"/><m:defJc m:val="right"/><m:wrapIndent m:val="2880"/><m:wrapRight/>', '', '<w:jc w:val="left"/>'),
    ('disabled-default-style', '<m:dispDef m:val="0"/>', '<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:pPr><w:jc w:val="center"/></w:pPr></w:style>', ''),
    ('disabled-document-default', '<m:dispDef m:val="0"/>', '', ''),
    ('disabled-inherited-rtl', '<m:dispDef m:val="0"/>', '<w:style w:type="paragraph" w:styleId="Base"><w:pPr><w:bidi/><w:jc w:val="end"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="Child"><w:basedOn w:val="Base"/></w:style>', '<w:pStyle w:val="Child"/>'),
    ('disabled-direct-override', '<m:dispDef m:val="0"/>', '<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:pPr><w:jc w:val="right"/></w:pPr></w:style>', '<w:jc w:val="left"/>'),
]


def equation(text, paragraph_properties, explicit=False):
    alignment = '<m:oMathParaPr><m:jc m:val="center"/></m:oMathParaPr>' if explicit else ''
    return f'<w:p><w:pPr>{paragraph_properties}</w:pPr><m:oMathPara>{alignment}<m:oMath><m:r><m:t>{text}</m:t></m:r></m:oMath></m:oMathPara></w:p>'


def generate():
    manifest = []
    for name, math_settings, styles, paragraph_properties in CASES:
        # Three separate display paragraphs: default alignment, explicit math
        # alignment, and enough break opportunities to expose continuation rules.
        body = '<w:p><w:r><w:t>before</w:t></w:r></w:p>'
        body += equation('x=1', paragraph_properties)
        body += equation('y=2', paragraph_properties, explicit=True)
        body += equation('z=' + '+'.join(['a', 'b', 'c', 'd'] * 20), paragraph_properties)
        body += '<w:p><w:r><w:t>after</w:t></w:r></w:p>'
        body += '<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>'
        default_alignment = '<w:jc w:val="right"/>' if name == 'disabled-document-default' else ''
        parts = {
            '[Content_Types].xml': '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/></Types>',
            '_rels/.rels': f'<Relationships xmlns="{REL}"><Relationship Id="rId1" Type="{OFFICE}/officeDocument" Target="word/document.xml"/></Relationships>',
            'word/_rels/document.xml.rels': f'<Relationships xmlns="{REL}"><Relationship Id="rId1" Type="{OFFICE}/styles" Target="styles.xml"/><Relationship Id="rId2" Type="{OFFICE}/settings" Target="settings.xml"/></Relationships>',
            'word/document.xml': f'<w:document xmlns:w="{W}" xmlns:m="{M}"><w:body>{body}</w:body></w:document>',
            'word/styles.xml': f'<w:styles xmlns:w="{W}"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Cambria" w:hAnsi="Cambria"/><w:sz w:val="24"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr>{default_alignment}<w:spacing w:before="80" w:after="80"/></w:pPr></w:pPrDefault></w:docDefaults>{styles}</w:styles>',
            'word/settings.xml': f'<w:settings xmlns:w="{W}" xmlns:m="{M}"><m:mathPr><m:mathFont m:val="Cambria Math"/>{math_settings}</m:mathPr></w:settings>',
        }
        path = ROOT / f'{name}.docx'
        with ZipFile(path, 'w', ZIP_DEFLATED) as archive:
            for part, text in parts.items():
                entry = ZipInfo(part, (2026, 11, 9, 0, 0, 0))
                entry.compress_type = ZIP_DEFLATED
                archive.writestr(entry, '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' + text)
        manifest.append({'file': path.name, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'math_settings': math_settings, 'paragraph_properties': paragraph_properties, 'styles': styles})
    (ROOT / 'sources.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    generate()
