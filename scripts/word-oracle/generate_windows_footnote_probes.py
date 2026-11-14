"""Reproduce 43 OOXML inputs measured by Windows Word 16.0.19127.20936.

No reference coordinates are used. The separate Word-authored observation is
preserved as captured; it is not reproduced by rewriting its XML.
"""
import argparse
import hashlib
from pathlib import Path
import re
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).resolve().parents[2]
SOURCES = {
    'footnote': 'eecd3c87d245794afb8a91116647fb8a79651d39343156d76eb82bb6618badc1',
    'footnote-long-continuation': '89839c9589cf3e4e246360340397bb20124331fcd40ebb0305a157d483dafbe2',
}


def load(name):
    source = ROOT / 'public/fixtures' / (name + '.docx')
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCES[name]:
        raise ValueError('Original probe input changed: ' + name)
    with ZipFile(source) as archive:
        return {name: archive.read(name) for name in archive.namelist()}


def write(output, name, parts):
    with ZipFile(output / (name + '.docx'), 'w', compression=ZIP_DEFLATED,
                 compresslevel=9) as archive:
        for member, data in sorted(parts.items()):
            info = ZipInfo(member, (2000, 1, 1, 0, 0, 0))
            info.compress_type = ZIP_DEFLATED
            archive.writestr(info, data)


def properties(face, size, complex_face=False):
    cs = f' w:cs="{face}"' if complex_face else ''
    return (f'<w:rPr><w:rFonts w:ascii="{face}" w:hAnsi="{face}" '
            f'w:eastAsia="{face}"{cs}/><w:sz w:val="{size}"/>'
            f'<w:szCs w:val="{size}"/></w:rPr>')


def rules(parts, normal, continuation, named=False):
    parts = dict(parts)
    def replace(match):
        ordinary = 'w:type="separator"' in match[0]
        face, size = normal if ordinary else continuation
        text = re.sub(r'<w:r>(?:<w:rPr>.*?</w:rPr>)?',
                      lambda _: '<w:r>' + properties(face, size), match[0], flags=re.S)
        if named:
            text = text.replace('<w:separator/>', '<w:t>SEPARATOR</w:t>')
            text = text.replace('<w:continuationSeparator/>', '<w:t>CONTINUATION</w:t>')
        return text
    text, count = re.subn(
        r'<w:footnote\b[^>]*w:type="(?:separator|continuationSeparator)"[^>]*>.*?</w:footnote>',
        replace, parts['word/footnotes.xml'].decode(), flags=re.S)
    assert count == 2
    parts['word/footnotes.xml'] = text.encode()
    return parts


def generate(output):
    output.mkdir(parents=True, exist_ok=False)
    variants = {}
    for source_name in SOURCES:
        original = load(source_name)
        for slug, face in [('arial', 'Arial'), ('century', 'Century'),
                           ('yu-mincho', 'Yu Mincho'), ('ms-mincho', 'MS Mincho')]:
            for size in (21, 24, 40):
                parts = dict(original)
                text, count = re.subn(
                    r'<w:footnote\b[^>]*w:type="(?:separator|continuationSeparator)"[^>]*>.*?</w:footnote>',
                    lambda match: re.sub(r'<w:r>(?:<w:rPr>.*?</w:rPr>)?',
                        lambda _: '<w:r>' + properties(face, size, True),
                        match[0], flags=re.S),
                    parts['word/footnotes.xml'].decode(), flags=re.S)
                assert count == 2
                parts['word/footnotes.xml'] = text.encode()
                name = f'word-windows-edge-{source_name}-rule-{slug}-size{size}-compat15'
                write(output, name, parts)
                variants[source_name, slug, size] = parts
    original = load('footnote-long-continuation')
    choices = {}
    for slug, normal, continuation, named in [
        ('arial21-yu40', ('Arial', 21), ('Yu Mincho', 40), False),
        ('yu40-arial21', ('Yu Mincho', 40), ('Arial', 21), False),
        ('named-stories', ('Arial', 21), ('Arial', 21), True),
    ]:
        parts = rules(original, normal, continuation, named)
        write(output, f'word-windows-edge-footnote-story-choice-{slug}-compat15', parts)
        choices[slug] = parts
    for margin in range(1128, 1141, 2):
        parts = dict(variants['footnote-long-continuation', 'ms-mincho', 40])
        text, count = re.subn(r'w:bottom="1134"', f'w:bottom="{margin}"',
                              parts['word/document.xml'].decode())
        assert count == 1
        parts['word/document.xml'] = text.encode()
        write(output, f'word-windows-edge-footnote-ms20-margin{margin}-compat15', parts)
    for slug, source in [('mixed-fonts', 'arial21-yu40'), ('named-stories', 'named-stories')]:
        parts = dict(choices[source])
        def join(match):
            paragraphs = re.findall(r'<w:p>.*?</w:p>', match[0], re.S)
            assert len(paragraphs) == 40
            first = paragraphs[0][:-6]
            following = ''.join(re.sub(r'^<w:p>(?:<w:pPr>.*?</w:pPr>)?', '', p)[:-6]
                                .replace('<w:r>', '<w:r><w:br/>', 1)
                                for p in paragraphs[1:])
            return match[0][:match[0].index('<w:p>')] + first + following + '</w:p></w:footnote>'
        text, count = re.subn(r'<w:footnote w:id="2">.*?</w:footnote>', join,
                              parts['word/footnotes.xml'].decode(), flags=re.S)
        assert count == 1
        parts['word/footnotes.xml'] = text.encode()
        write(output, f'word-windows-edge-footnote-inside-paragraph-{slug}-compat15', parts)
    texts = ['SS', 'EE', 'PP', 'AA', 'RR', 'TT', 'OO', 'SE', 'EP', 'PA', 'AR',
             'RA', 'AT', 'TO', 'OR', 'SEPARATOR', 'AV', 'To', 'Wa', 'Wo',
             'ABCDEFGHIJKLMNOPQRSTUVWXYZ']
    for size in (21, 24, 40):
        for kern in (0, 2):
            parts = dict(original)
            props = ('<w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="Arial" w:cs="Arial"/>'
                     f'<w:sz w:val="{size}"/><w:szCs w:val="{size}"/><w:kern w:val="{kern}"/>')
            body = ''.join('<w:p><w:pPr><w:jc w:val="left"/>'
                           '<w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/>'
                           '<w:snapToGrid w:val="0"/></w:pPr>'
                           f'<w:r><w:rPr>{props}</w:rPr><w:t>{text}</w:t></w:r></w:p>'
                           for text in texts)
            text, count = re.subn(r'(<w:body>).*?(<w:sectPr)',
                                  lambda match: match[1] + body + match[2],
                                  parts['word/document.xml'].decode(), flags=re.S)
            assert count == 1
            parts['word/document.xml'] = re.sub(r'<w:docGrid\b[^>]*/>',
                '<w:docGrid w:type="default" w:linePitch="360"/>', text).encode()
            write(output, f'word-windows-edge-arial-kern{kern}-size{size}-compat15', parts)
    parts = dict(original)
    def repeat_note(match):
        paragraphs = re.findall(r'<w:p>.*?</w:p>', match[0], re.S)
        assert len(paragraphs) == 40
        return match[0][:-13] + ''.join(paragraphs[1:]) * 3 + '</w:footnote>'
    text, count = re.subn(r'<w:footnote w:id="2">.*?</w:footnote>', repeat_note,
                          parts['word/footnotes.xml'].decode(), flags=re.S)
    assert count == 1
    parts['word/footnotes.xml'] = text.encode()
    text = parts['word/document.xml'].decode()
    section, = re.findall(r'<w:sectPr>.*?</w:sectPr>', text, re.S)
    body = re.search(r'<w:body>(.*?)<w:sectPr>', text, re.S)[1]
    paragraphs = re.findall(r'<w:p>.*?</w:p>', body, re.S)
    assert len(paragraphs) == 40
    reference = paragraphs[30]
    if '<w:pPr>' in reference:
        updated = reference.replace('</w:pPr>', section + '</w:pPr>', 1)
    else:
        updated = reference.replace('<w:p>', '<w:p><w:pPr>' + section + '</w:pPr>', 1)
    text = text.replace(reference, updated, 1)
    incoming = section.replace('<w:sectPr>', '<w:sectPr><w:type w:val="nextPage"/>')
    incoming = incoming.replace('w:top="1134"', 'w:top="6000"')
    offset = text.rfind(section)
    assert offset >= 0
    parts['word/document.xml'] = (text[:offset] + text[offset:].replace(section, incoming, 1)).encode()
    write(output, 'word-windows-edge-footnote-long-carry-section-margin300-compat15', parts)
    return 43


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-directory', type=Path, required=True)
    args = parser.parse_args()
    print('Generated', generate(args.output_directory), 'Windows Word probe inputs')
