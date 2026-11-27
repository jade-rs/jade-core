"""Generate Word probes for manual breaks and tabs beside a floating table."""

from copy import copy
from zipfile import ZIP_DEFLATED, ZipFile

from generate_windows_floating_terminal_run_cases import FIXTURES, PARAGRAPH, TARGET
from generate_windows_page_break_cases import MODES

PREFIX = 'word-windows-edge-floating-line-ending-'
BREAK_GAPS = (70, 80, 90, 100)
TAB_GAPS = (80, 100)
LINE_ENDINGS = {
    'break-dot': '<w:r><w:t>.</w:t><w:br/><w:t>Next line.</w:t></w:r>',
    'break-comma': '<w:r><w:t>,</w:t><w:br/><w:t>Next line.</w:t></w:r>',
    'break-dot-space': ('<w:r><w:t xml:space="preserve">. </w:t></w:r>'
                        '<w:r><w:br/><w:t>Next line.</w:t></w:r>'),
    'break-plain': '<w:r><w:br/><w:t>Next line.</w:t></w:r>',
    'terminal-break-dot': '<w:r><w:t>.</w:t><w:br/></w:r>',
    'carriage-return-dot': '<w:r><w:t>.</w:t><w:cr/><w:t>Next line.</w:t></w:r>',
    'terminal-tab-dot': '<w:r><w:t>.</w:t><w:tab/></w:r>',
    'terminal-bold-space-dot': ('<w:r><w:t>.</w:t></w:r><w:r><w:rPr><w:b/></w:rPr>'
                                '<w:t xml:space="preserve"> </w:t></w:r>'),
}
TAB_ENDINGS = {
    'tab-comma': '<w:r><w:t xml:space="preserve">,</w:t><w:tab/></w:r>',
    'tab-plain': '<w:r><w:tab/></w:r>',
    'tab-next-word': ('<w:r><w:t xml:space="preserve">.</w:t>'
                      '<w:tab/><w:t>NEXT</w:t></w:r>'),
    'space-tab': '<w:r><w:t xml:space="preserve">. </w:t><w:tab/></w:r>',
    'tab-space': ('<w:r><w:t xml:space="preserve">.</w:t>'
                  '<w:tab/><w:t xml:space="preserve"> </w:t></w:r>'),
    'two-tabs': '<w:r><w:t xml:space="preserve">.</w:t><w:tab/><w:tab/></w:r>',
    'right-tab-dot': '<w:r><w:t xml:space="preserve">.</w:t><w:tab/></w:r>',
    'tab-semicolon': '<w:r><w:t xml:space="preserve">;</w:t><w:tab/></w:r>',
}


def cases():
    return {f'{name}-gap-{gap}' for endings, gaps in ((LINE_ENDINGS, BREAK_GAPS),
                                                     (TAB_ENDINGS, TAB_GAPS))
            for name in endings for gap in gaps}


def generate(output=FIXTURES):
    output.mkdir(parents=True, exist_ok=True)
    for mode in MODES:
        for gap in BREAK_GAPS:
            template = FIXTURES / (
                f'word-windows-edge-floating-line-fit-probe-left-dot-gap-{gap}-compat{mode}.docx'
            )
            endings = LINE_ENDINGS | TAB_ENDINGS if gap in TAB_GAPS else LINE_ENDINGS
            with ZipFile(template) as source:
                document = source.read('word/document.xml').decode('utf-8')
                if document.count(TARGET) != 1:
                    raise ValueError(f'{template}: expected the split-run terminal period probe')
                for name, terminal in endings.items():
                    alignment = 'right' if name == 'right-tab-dot' else 'left'
                    paragraph = PARAGRAPH.format(align=alignment, terminal=terminal)
                    changed = document.replace(TARGET, paragraph).encode('utf-8')
                    path = output / f'{PREFIX}{name}-gap-{gap}-compat{mode}.docx'
                    with ZipFile(path, 'w', ZIP_DEFLATED) as target:
                        for entry in source.infolist():
                            data = changed if entry.filename == 'word/document.xml' else source.read(entry)
                            target.writestr(copy(entry), data)


if __name__ == '__main__':
    generate()
