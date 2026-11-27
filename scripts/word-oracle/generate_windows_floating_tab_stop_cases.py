"""Generate tab-grid and authored-stop probes in a floating-table text frame."""

from copy import copy
from zipfile import ZIP_DEFLATED, ZipFile

from generate_windows_floating_terminal_run_cases import FIXTURES, TARGET
from generate_windows_page_break_cases import MODES

PREFIX = 'word-windows-edge-floating-tab-stop-'
GAPS = (70, 80, 90, 100)
TEXT = 'Filler paragraph 8 gives the floating table text to wrap around while the page fills'
DEFAULT_TAB = '<w:defaultTabStop w:val="720"/>'

# (default interval in twips, authored stops, paragraph alignment, text after tab)
VARIANTS = {
    'default-24': (480, '', 'left', 'NEXT'),
    'default-48': (960, '', 'left', 'NEXT'),
    'default-24-right-terminal': (480, '', 'right', ''),
    'default-48-right-terminal': (960, '', 'right', ''),
    'left-near': (720, '<w:tab w:val="left" w:pos="720"/>', 'left', 'NEXT'),
    'left-middle': (720, '<w:tab w:val="left" w:pos="3000"/>', 'left', 'NEXT'),
    'left-far': (720, '<w:tab w:val="left" w:pos="8400"/>', 'left', 'NEXT'),
    'right-middle': (720, '<w:tab w:val="right" w:pos="3000"/>', 'left', 'NEXT'),
    'center-middle': (720, '<w:tab w:val="center" w:pos="3000"/>', 'left', 'NEXT'),
    'decimal-middle': (720, '<w:tab w:val="decimal" w:pos="3000"/>', 'left', '25.3'),
    'leader-middle': (720, '<w:tab w:val="left" w:leader="dot" w:pos="3000"/>', 'left', 'NEXT'),
    'left-middle-right-terminal': (720, '<w:tab w:val="left" w:pos="3000"/>', 'right', ''),
}


def variants_for_gap(gap):
    return ((name, variant) for name, variant in VARIANTS.items()
            if gap in (80, 100) or variant[1])


def cases():
    return {f'{case}-gap-{gap}' for gap in GAPS for case, _ in variants_for_gap(gap)}


def generate(output=FIXTURES):
    output.mkdir(parents=True, exist_ok=True)
    for mode in MODES:
        for gap in GAPS:
            name = f'word-windows-edge-floating-line-fit-probe-left-dot-gap-{gap}-compat{mode}.docx'
            with ZipFile(FIXTURES / name) as source:
                doc = source.read('word/document.xml').decode()
                settings = source.read('word/settings.xml').decode()
                if doc.count(TARGET) != 1 or settings.count(DEFAULT_TAB) != 1:
                    raise ValueError(f'{name}: unexpected floating-table template')
                for variant, (interval, authored, align, suffix) in variants_for_gap(gap):
                    tabs = f'<w:tabs>{authored}</w:tabs>' if authored else ''
                    tail = f'<w:t>{suffix}</w:t>' if suffix else ''
                    paragraph = (f'<w:p><w:pPr>{tabs}<w:jc w:val="{align}"/></w:pPr>'
                                 f'<w:r><w:t>{TEXT}</w:t></w:r><w:r><w:t>.</w:t><w:tab/>'
                                 f'{tail}</w:r></w:p>')
                    parts = {
                        'word/document.xml': doc.replace(TARGET, paragraph).encode(),
                        'word/settings.xml': settings.replace(
                            DEFAULT_TAB, f'<w:defaultTabStop w:val="{interval}"/>'
                        ).encode(),
                    }
                    path = output / f'{PREFIX}{variant}-gap-{gap}-compat{mode}.docx'
                    with ZipFile(path, 'w', ZIP_DEFLATED) as result:
                        for part in source.infolist():
                            data = parts[part.filename] if part.filename in parts else source.read(part)
                            result.writestr(copy(part), data)


if __name__ == '__main__':
    generate()
