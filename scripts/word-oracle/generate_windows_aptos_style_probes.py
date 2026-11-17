"""Reproduce standalone and mixed Aptos style controls from the Mac original."""
import argparse
from copy import deepcopy
from io import BytesIO
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

from lxml import etree as ET

ROOT = Path(__file__).resolve().parents[2]
PARENT = Path('tests/word-oracle/mac-elsa-synthetic/aptos-ascii-advances-mac/source.docx')
W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'


def probes(root=ROOT):
    ET.register_namespace('w', W)
    q = lambda name: '{' + W + '}' + name
    with ZipFile(root / PARENT) as parent:
        original = ET.fromstring(parent.read('word/document.xml'))
        body = original.find(q('body'))
        template, section = body.find(q('p')), body.find(q('sectPr'))
        for mixed in (False, True):
            for family, label in [('Aptos', 'aptos'), ('Aptos Display', 'display')]:
                for size in (12, 20, 40):
                    for style, bold, italic in [
                        ('regular', False, False), ('italic', False, True),
                        ('bold', True, False), ('bolditalic', True, True),
                    ]:
                        document = deepcopy(original)
                        output = document.find(q('body'))
                        output.clear()
                        for index in range(3 if mixed else 2):
                            paragraph = deepcopy(template)
                            run = paragraph.find(q('r'))
                            props = run.find(q('rPr'))
                            active = not mixed or index > 0
                            for attribute in ('ascii', 'hAnsi'):
                                props.find(q('rFonts')).set(q(attribute), family)
                            props.find(q('sz')).set(q('val'), str(size * 2))
                            props.find(q('b')).set(q('val'), '1' if bold and active else '0')
                            props.find(q('i')).set(q('val'), '1' if italic and active else '0')
                            run.find(q('t')).text = 'AV To 11x'
                            output.append(paragraph)
                        output.append(deepcopy(section))
                        xml = ET.tostring(document, encoding='utf-8', xml_declaration=True)
                        buffer = BytesIO()
                        with ZipFile(buffer, 'w', compression=ZIP_DEFLATED) as bundle:
                            for entry in parent.infolist():
                                item = ZipInfo(entry.filename, (2026, 11, 17, 0, 0, 0))
                                item.create_system = 0
                                item.compress_type = ZIP_DEFLATED
                                item.external_attr = entry.external_attr
                                bundle.writestr(item, xml if entry.filename == 'word/document.xml'
                                                else parent.read(entry.filename))
                        layout = 'mixed-style' if mixed else 'style'
                        name = f'word-windows-aptos-{layout}-{label}-size{size}-{style}-compat14'
                        yield name, buffer.getvalue()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output_directory', type=Path)
    args = parser.parse_args()
    args.output_directory.mkdir(parents=True, exist_ok=False)
    for name, raw in probes():
        (args.output_directory / (name + '.docx')).write_bytes(raw)
