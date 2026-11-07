"""Deterministic Word layout probes; render originals without resaving in Word."""
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
from xml.sax.saxutils import escape

ROOT = Path(__file__).resolve().parents[2]
NS = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'


def run(text, properties=''):
    return f'<w:r><w:rPr>{properties}</w:rPr><w:t xml:space="preserve">{escape(text)}</w:t></w:r>'


def para(content, properties=''):
    return '<w:p><w:pPr><w:snapToGrid w:val="0"/>' + properties + '</w:pPr>' + content + '</w:p>'


def write(name, body):
    parts = {
        '[Content_Types].xml': '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>' + ''.join(f'<Override PartName="/word/{p}.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.{t}+xml"/>' for p, t in [('document', 'document.main'), ('styles', 'styles'), ('settings', 'settings')]) + '</Types>',
        '_rels/.rels': '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>',
        'word/_rels/document.xml.rels': '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' + ''.join(f'<Relationship Id="rId{i}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{p}" Target="{p}.xml"/>' for i, p in enumerate(['styles', 'settings'], 1)) + '</Relationships>',
        'word/styles.xml': f'<w:styles xmlns:w="{NS}"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="MS Mincho"/><w:sz w:val="24"/><w:lang w:val="en-US" w:eastAsia="ja-JP"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style></w:styles>',
        'word/settings.xml': f'<w:settings xmlns:w="{NS}"><w:defaultTabStop w:val="720"/><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>',
        'word/document.xml': f'<w:document xmlns:w="{NS}"><w:body>{body}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr></w:body></w:document>',
    }
    target = ROOT / 'public/fixtures' / f'word-windows-edge-{name}.docx'
    with ZipFile(target, 'w') as archive:
        for path, xml in parts.items():
            item = ZipInfo(path, (2026, 11, 7, 0, 0, 0))
            item.compress_type = ZIP_DEFLATED
            archive.writestr(item, xml.encode('utf-8'))
    print(target)


def main():
    paragraphs = []
    for align in ['left', 'center', 'right', 'both']:
        for first in ['w:firstLine="360"', 'w:hanging="360"', 'w:firstLine="-360"']:
            paragraphs.append(para(run('Alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu. ' * 3), f'<w:jc w:val="{align}"/><w:ind w:left="720" w:right="360" {first}/><w:spacing w:after="120"/>'))
    write('indents', ''.join(paragraphs))
    paragraphs = []
    for align in ['left', 'center', 'right', 'decimal']:
        for text in ['12.34', '1234.5', 'AB CD']:
            paragraphs.append(para(run('Start') + '<w:r><w:tab/></w:r>' + run(text) + '<w:r><w:tab/></w:r>' + run('End'), f'<w:tabs><w:tab w:val="{align}" w:pos="2880"/><w:tab w:val="left" w:pos="5760"/></w:tabs>'))
    write('tabs', ''.join(paragraphs))
    paragraphs = []
    for before, after in [(0, 0), (120, 240), (240, 120), (360, 360)]:
        for contextual in ['', '<w:contextualSpacing/>']:
            for row in range(3):
                paragraphs.append(para(run(f'Spacing {before} {after} row {row}'), f'<w:spacing w:before="{before}" w:after="{after}"/>{contextual}'))
    write('spacing', ''.join(paragraphs))
    paragraphs = []
    for size in [20, 24, 40]:
        for effect in ['<w:position w:val="6"/>', '<w:position w:val="-6"/>', '<w:vertAlign w:val="superscript"/>', '<w:vertAlign w:val="subscript"/>']:
            paragraphs.append(para(run('Base ', f'<w:sz w:val="{size}"/>') + run('Shift', f'<w:sz w:val="{size}"/>{effect}') + run(' Tail', f'<w:sz w:val="{size}"/>')))
    write('run-position', ''.join(paragraphs))
    paragraphs = []
    for size in [20, 24, 40]:
        for spacing in [-10, 0, 10, 30]:
            for scale in [80, 100, 150]:
                paragraphs.append(para(run('ABCDE abcde 01234', f'<w:sz w:val="{size}"/><w:spacing w:val="{spacing}"/><w:w w:val="{scale}"/>')))
    write('run-width', ''.join(paragraphs))


if __name__ == '__main__':
    main()
