"""Reconstruct Windows script metrics from original Word output and source runs."""
import hashlib
from pathlib import Path
from zipfile import ZipFile
import xml.etree.ElementTree as ET

from check_platform_coordinates import ROOT, read_json, require
from extract_windows_emf import extract

DIRECTORY = Path('tests/word-oracle/windows/word-16.0.19127.20936/20261114T050826030Z-cc248634')
METRICS_SHA256 = 'd763babddeb14ab78f74f94bd2dca4e8d98ed9bfa96f46885d3fb4426294be32'
W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def verify(root=ROOT):
    import pymupdf
    directory = root / DIRECTORY
    metrics = directory / 'script-metrics.json'
    require(hashlib.sha256(metrics.read_bytes()).hexdigest() == METRICS_SHA256,
            'Windows script metric coverage changed')
    expected = read_json(metrics)
    capture = read_json(directory / 'capture.json')
    require(expected['word'] == capture['word']
            and expected['capture_id'] == capture['capture_id'], 'Script provenance changed')
    require(read_json(directory / 'word-emf-reference.json') == extract(directory / 'capture.json'),
            'Original script drawing evidence changed')
    facts, documents = [], []
    for document in capture['documents']:
        source = directory / 'inputs' / (document['name'] + '.docx')
        require(source.read_bytes() == (root / 'public/fixtures' / source.name).read_bytes(),
                'Script source differs from captured input')
        require(hashlib.sha256(source.read_bytes()).hexdigest() == document['sha256'],
                'Script source changed')
        with ZipFile(source) as package:
            paragraphs = ET.fromstring(package.read('word/document.xml')).findall(f'{W}body/{W}p')
        spans = []
        with pymupdf.open(directory / document['pdf']) as pdf:
            for page in pdf:
                for block in page.get_text('dict')['blocks']:
                    for line in block.get('lines', []):
                        spans.extend(span for span in line['spans'] if span['text'].strip())
        require(len(paragraphs) == 392 and len(spans) == 392 * 3, 'Script row coverage changed')
        for index, paragraph in enumerate(paragraphs):
            runs = paragraph.findall(f'{W}r')
            normal, script = runs[0].find(f'{W}rPr'), runs[1].find(f'{W}rPr')
            first, shifted, last = spans[index * 3:index * 3 + 3]
            require(tuple(s['text'].strip() for s in (first, shifted, last)) == ('A', '12', 'B'),
                    'Script probe text changed')
            require(abs(first['origin'][1] - last['origin'][1]) < .001, 'Split normal baseline')
            kind = script.find(f'{W}vertAlign').get(f'{W}val')
            move = (first['origin'][1] - shifted['origin'][1]) * (1 if kind == 'superscript' else -1)
            facts.append(dict(
                fixture=document['name'], family=normal.find(f'{W}rFonts').get(f'{W}ascii'),
                size_pt=float(normal.find(f'{W}sz').get(f'{W}val')) / 2, kind=kind,
                bold=normal.find(f'{W}b') is not None, italic=normal.find(f'{W}i') is not None,
                word_font=shifted['font'], actual_script_size=shifted['size'],
                script_size_pt=round(shifted['size'] * 2) / 2,
                move_pt=round(move * 2) / 2, observed_move=move,
            ))
        documents.append(dict(name=document['name'], source_sha256=document['sha256'],
                              pdf_sha256=document['pdf_sha256']))
    require(expected['documents'] == documents and expected['facts'] == facts,
            'Script metrics differ from original Word PDF/source')
    require(len(facts) == 1568, 'Script metric coverage removed')
    return len(facts)


if __name__ == '__main__':
    print(f'Verified {verify()} Windows Word font/size/style/script measurements')
