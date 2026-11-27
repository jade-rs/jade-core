"""Generate paragraphs with successive tab characters beside a floating table."""

from copy import copy
from xml.sax.saxutils import escape
from zipfile import ZIP_DEFLATED, ZipFile

from generate_windows_floating_tab_stop_cases import DEFAULT_TAB, FIXTURES, MODES, TARGET, TEXT

PREFIX = 'word-windows-edge-floating-multi-tab-'
GAPS = (80, 100)

# (authored stops as position, alignment and leader, then text after each tab)
VARIANTS = {
    'default': ((), ('A', 'B')),
    'default-terminal': ((), ('A', '')),
    'two-left': (((900, 'left', ''), (3000, 'left', '')), ('A', 'B')),
    'implicit-after-authored': (((1400, 'left', ''),), ('ABC', 'B')),
    'right-decimal': (((1800, 'right', ''), (3300, 'decimal', '')), ('ALPHA', '25.3')),
    'two-dot-leaders': (((1600, 'left', 'dot'), (3300, 'right', 'dot')), ('A', 'BETA')),
    'far-second': (((1800, 'left', ''), (8400, 'left', '')), ('A', 'NEXT')),
    'far-first': (((8400, 'left', ''),), ('A', 'B')),
    'far-adjacent': (((8400, 'left', ''),), ('', 'B')),
    'far-after-wide': (((8400, 'left', ''),), ('W', 'B')),
    'far-after-two-wide': (((8400, 'left', ''),), ('MM', 'B')),
}


def cases():
    return {f'{name}-gap-{gap}' for name in VARIANTS for gap in GAPS}


def paragraph_xml(stops, tail):
    tabs = []
    for position, alignment, leader in stops:
        leader_attr = f' w:leader="{leader}"' if leader else ''
        tabs.append(f'<w:tab w:val="{alignment}" w:pos="{position}"{leader_attr}/>')
    properties = f'<w:tabs>{"".join(tabs)}</w:tabs>' if tabs else ''
    suffix = ''.join('<w:tab/>' + (f'<w:t>{escape(value)}</w:t>' if value else '') for value in tail)
    return (f'<w:p><w:pPr>{properties}<w:jc w:val="left"/></w:pPr>'
            f'<w:r><w:t>{TEXT}</w:t></w:r><w:r><w:t>.</w:t>{suffix}</w:r></w:p>')


def generate(output=FIXTURES):
    output.mkdir(parents=True, exist_ok=True)
    for mode in MODES:
        for gap in GAPS:
            template = f'word-windows-edge-floating-line-fit-probe-left-dot-gap-{gap}-compat{mode}.docx'
            with ZipFile(FIXTURES / template) as source:
                document = source.read('word/document.xml').decode()
                settings = source.read('word/settings.xml').decode()
                if document.count(TARGET) != 1 or settings.count(DEFAULT_TAB) != 1:
                    raise ValueError(f'{template}: unexpected floating-table template')
                for name, (stops, tail) in VARIANTS.items():
                    contents = document.replace(TARGET, paragraph_xml(stops, tail)).encode()
                    path = output / f'{PREFIX}{name}-gap-{gap}-compat{mode}.docx'
                    with ZipFile(path, 'w', ZIP_DEFLATED) as target:
                        for part in source.infolist():
                            data = contents if part.filename == 'word/document.xml' else source.read(part)
                            target.writestr(copy(part), data)


if __name__ == '__main__':
    generate()
