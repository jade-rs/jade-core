"""Reproduce isolated fixed-pitch Japanese/Latin boundary controls."""
import argparse
from copy import deepcopy
from io import BytesIO
from pathlib import Path
from xml.etree import ElementTree as ET
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

ROOT = Path(__file__).resolve().parents[2]
PARENT = Path('tests/word-oracle/mac-elsa-synthetic/word-mac-probe-exact-grid/source.docx')
W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'


def probes(root=ROOT):
    ET.register_namespace('w', W)
    q = lambda name: '{' + W + '}' + name
    with ZipFile(root / PARENT) as parent:
        original = ET.fromstring(parent.read('word/document.xml'))
        body = original.find(q('body'))
        template, section = body.find(q('p')), body.find(q('sectPr'))
        for family, label in [('ＭＳ 明朝', 'mincho'), ('ＭＳ ゴシック', 'gothic')]:
            for size in [10.5, 11, 20]:
                controls = [(a, b, enabled)
                            for a, b in [('亜', 'A'), ('A', '亜')]
                            for enabled in [None, True, False]]
                controls += [(a, b, None) for a, b in [('亜', '1'), ('1', '亜')]]
                for first, second, enabled in controls:
                    document = deepcopy(original)
                    output_body = document.find(q('body'))
                    output_body.clear()
                    paragraph = deepcopy(template)
                    if enabled is not None:
                        ET.SubElement(paragraph.find(q('pPr')), q('autoSpaceDE'),
                                      {q('val'): '1' if enabled else '0'})
                    run = paragraph.find(q('r'))
                    properties = run.find(q('rPr'))
                    for attribute in ['ascii', 'eastAsia', 'hAnsi']:
                        properties.find(q('rFonts')).set(q(attribute), family)
                    properties.find(q('sz')).set(q('val'), str(int(size * 2)))
                    run.find(q('t')).text = first + second
                    output_body.append(paragraph)
                    output_body.append(deepcopy(section))
                    if first == '亜':
                        direction = 'cjk-latin' if second == 'A' else 'cjk-digit'
                    else:
                        direction = 'latin-cjk' if first == 'A' else 'digit-cjk'
                    mode = 'implicit' if enabled is None else ('enabled' if enabled else 'disabled')
                    size_name = str(size).replace('.', 'p')
                    name = f'word-windows-ms-{label}-size{size_name}-{direction}-de-{mode}-compat15'
                    xml = ET.tostring(document, encoding='utf-8', xml_declaration=True)
                    buffer = BytesIO()
                    with ZipFile(buffer, 'w', compression=ZIP_DEFLATED, compresslevel=6) as target:
                        for info in parent.infolist():
                            item = ZipInfo(info.filename, (2026, 11, 17, 0, 0, 0))
                            item.create_system = 0
                            item.compress_type = ZIP_DEFLATED
                            item.external_attr = info.external_attr
                            target.writestr(item, xml if info.filename == 'word/document.xml'
                                            else parent.read(info.filename))
                    yield name, buffer.getvalue()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output_directory', type=Path)
    args = parser.parse_args()
    args.output_directory.mkdir(parents=True, exist_ok=False)
    for name, data in probes():
        (args.output_directory / (name + '.docx')).write_bytes(data)
