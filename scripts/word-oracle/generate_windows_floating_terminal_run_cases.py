"""Vary the final text run in the floating-table boundary fixture."""

from copy import copy
from pathlib import Path
from zipfile import ZIP_DEFLATED, ZipFile

from generate_windows_page_break_cases import MODES


PREFIX = 'word-windows-edge-floating-terminal-run-'
FIXTURES = Path(__file__).resolve().parents[2] / 'public/fixtures'
GAPS = (80, 100)
VARIANTS = {
    'right-dot': ('right', '<w:r><w:t>.</w:t></w:r>'),
    'right-comma': ('right', '<w:r><w:t>,</w:t></w:r>'),
    'colored-dot': ('left', '<w:r><w:rPr><w:color w:val="AA0000"/></w:rPr><w:t>.</w:t></w:r>'),
    'italic-dot': ('left', '<w:r><w:rPr><w:i/></w:rPr><w:t>.</w:t></w:r>'),
    'empty-run-after-dot': ('left', '<w:r><w:t>.</w:t></w:r><w:r/>'),
    'bookmark-after-dot': ('left', '<w:r><w:t>.</w:t></w:r>'
                           '<w:bookmarkStart w:id="1" w:name="AfterDot"/><w:bookmarkEnd w:id="1"/>'),
    'space-after-dot': ('left', '<w:r><w:t xml:space="preserve">. </w:t></w:r>'),
    'separate-space-after-dot': ('left', '<w:r><w:t>.</w:t></w:r>'
                                '<w:r><w:t xml:space="preserve"> </w:t></w:r>'),
    'spaces-after-comma': ('left', '<w:r><w:t xml:space="preserve">,   </w:t></w:r>'),
    'right-space-after-dot': ('right', '<w:r><w:t xml:space="preserve">. </w:t></w:r>'),
    'nbsp-after-dot': ('left', '<w:r><w:t>.&#160;</w:t></w:r>'),
    'double-dot': ('left', '<w:r><w:t>..</w:t></w:r>'),
}
PARAGRAPH = ('<w:p><w:pPr><w:jc w:val="{align}"/></w:pPr><w:r><w:t>Filler paragraph 8 gives '
             'the floating table text to wrap around while the page fills</w:t></w:r>'
             '{terminal}</w:p>')
TARGET = PARAGRAPH.format(align='left', terminal='<w:r><w:t>.</w:t></w:r>')


def cases():
    return {f'{name}-gap-{gap}' for name in VARIANTS for gap in GAPS}


def generate(output=FIXTURES):
    output.mkdir(parents=True, exist_ok=True)
    for mode in MODES:
        for gap in GAPS:
            template = FIXTURES / (
                f'word-windows-edge-floating-line-fit-probe-left-dot-gap-{gap}-compat{mode}.docx'
            )
            with ZipFile(template) as source:
                raw = source.read('word/document.xml').decode('utf-8')
                if raw.count(TARGET) != 1:
                    raise ValueError(f'{template}: expected a single explicitly left-aligned probe')
                for name, (align, terminal) in VARIANTS.items():
                    paragraph = PARAGRAPH.format(align=align, terminal=terminal)
                    document = raw.replace(TARGET, paragraph).encode('utf-8')
                    with ZipFile(output / f'{PREFIX}{name}-gap-{gap}-compat{mode}.docx',
                                 'w', ZIP_DEFLATED) as target:
                        for entry in source.infolist():
                            data = document if entry.filename == 'word/document.xml' else source.read(entry)
                            target.writestr(copy(entry), data)


if __name__ == '__main__':
    generate()
