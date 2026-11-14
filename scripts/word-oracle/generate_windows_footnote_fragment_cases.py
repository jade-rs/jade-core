"""Vary body length and split-note length in the unchanged Mac original."""
import hashlib
from pathlib import Path
import re
from zipfile import ZipFile

from generate_windows_script_cases import write

ROOT = Path(__file__).resolve().parents[2]
SOURCE_SHA256 = '89839c9589cf3e4e246360340397bb20124331fcd40ebb0305a157d483dafbe2'


def main():
    fixtures = ROOT / 'public/fixtures'
    source = fixtures / 'footnote-long-continuation.docx'
    if hashlib.sha256(source.read_bytes()).hexdigest() != SOURCE_SHA256:
        raise ValueError('Original split-footnote source changed')
    with ZipFile(source) as archive:
        original = {name: archive.read(name) for name in archive.namelist()}
    cases = [(body, note) for body in (28, 30, 32, 35) for note in (20, 40)]
    cases += [(0, 60), (45, 60)]
    for body_count, note_count in cases:
        parts = dict(original)
        text = parts['word/document.xml'].decode()
        pattern = (r'<w:p><w:r><w:t xml:space="preserve">Body paragraph '
                   r'[0-9]+\.</w:t></w:r></w:p>')
        matches = list(re.finditer(pattern, text))
        if len(matches) != 30:
            raise ValueError('Original body paragraph coverage changed')
        paragraphs = ''.join(
            f'<w:p><w:r><w:t xml:space="preserve">Body paragraph {i}.</w:t></w:r></w:p>'
            for i in range(1, body_count + 1))
        text = text[:matches[0].start()] + paragraphs + text[matches[-1].end():]
        parts['word/document.xml'] = text.encode()
        text = parts['word/footnotes.xml'].decode()
        match = re.search(r'<w:footnote w:id="2">(.*?)</w:footnote>', text)
        if match is None:
            raise ValueError('Original note missing')
        paragraphs = re.findall(r'<w:p>.*?</w:p>', match[1])
        if len(paragraphs) != 40:
            raise ValueError('Original note paragraph coverage changed')
        replacement = paragraphs[0] + ''.join(
            paragraphs[1].replace('line 2.', f'line {i}.')
            for i in range(2, note_count + 1))
        parts['word/footnotes.xml'] = (
            text[:match.start(1)] + replacement + text[match.end(1):]).encode()
        name = f'word-windows-edge-footnote-fragment-body{body_count}-notes{note_count}-compat15'
        write(fixtures / (name + '.docx'), parts)
    print('Generated 10 split-footnote probes')


if __name__ == '__main__':
    main()
