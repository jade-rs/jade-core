#!/usr/bin/env python3
"""Extract original glyph origins without baseline grouping. Requires PyMuPDF."""
from pathlib import Path
import hashlib
import json
import fitz
root = Path(__file__).parent
pdf = root / 'arial-script-sweep-word.pdf'
source = root / 'arial-script-sweep.docx'
doc = fitz.open(pdf)
result = {
    'application': 'Microsoft Word for Mac 16.114.3',
    'export': 'AppleScript save as file format format PDF',
    'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
    'pdf_sha256': hashlib.sha256(pdf.read_bytes()).hexdigest(),
    'pymupdf': fitz.VersionBind,
    'pages': [],
}
for page in doc:
    spans = []
    for block in page.get_text('rawdict')['blocks']:
        for line in block.get('lines', []):
            for span in line['spans']:
                spans.append({
                    'text': ''.join(c['c'] for c in span['chars']),
                    'font': span['font'], 'size': span['size'],
                    'glyphs': [{'text': c['c'], 'origin': c['origin']} for c in span['chars']],
                })
    result['pages'].append({'size_pt': [page.rect.width, page.rect.height], 'spans': spans})
(root / 'word-glyphs.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
