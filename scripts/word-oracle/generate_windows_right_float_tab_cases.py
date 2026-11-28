"""Mirror Windows tab fixtures so the floating table reduces the right edge."""

from copy import copy
from zipfile import ZIP_DEFLATED, ZipFile

import generate_windows_floating_multi_tab_cases as multi
import generate_windows_floating_tab_stop_cases as single

PREFIX = 'word-windows-edge-right-float-tab-'
SINGLE_VARIANTS = ('left-far', 'left-middle-right-terminal', 'leader-middle')
ANCHOR = b'w:tblpXSpec="left"'


def variants():
    for case in sorted(multi.cases()):
        yield f'multi-{case}', multi.PREFIX + case
    for name in SINGLE_VARIANTS:
        for gap in multi.GAPS:
            case = f'{name}-gap-{gap}'
            yield f'single-{case}', single.PREFIX + case


def cases():
    return {case for case, _ in variants()}


def generate(output=single.FIXTURES):
    output.mkdir(parents=True, exist_ok=True)
    for mode in single.MODES:
        for case, template in variants():
            name = f'{template}-compat{mode}.docx'
            with ZipFile(single.FIXTURES / name) as source:
                document = source.read('word/document.xml')
                if document.count(ANCHOR) != 1:
                    raise ValueError(f'{name}: expected one left-anchored table')
                document = document.replace(ANCHOR, b'w:tblpXSpec="right"')
                path = output / f'{PREFIX}{case}-compat{mode}.docx'
                with ZipFile(path, 'w', ZIP_DEFLATED) as target:
                    for part in source.infolist():
                        data = document if part.filename == 'word/document.xml' else source.read(part)
                        target.writestr(copy(part), data)


if __name__ == '__main__':
    generate()
