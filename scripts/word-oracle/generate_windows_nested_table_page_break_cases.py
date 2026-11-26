"""Generate Word probes for page breaks and painting in fixed-width nested tables."""

from generate_windows_table_page_break_cases import (
    BREAK_BEFORE, MODES, PAGE, PAGE_GEOMETRY, STYLES, edge, para, run, table, text_run,
)

PREFIX = 'word-windows-edge-nested-table-width-page-break-'


def cases():
    first = para(run('FIRST'))
    side = para(run('SIDE'))
    head = para(run('HEAD'))
    tail = para(run('TAIL'))
    break_cases = {
        'paragraph-break-before': para(run('TARGET'), BREAK_BEFORE),
        'same-run-break': para(text_run('PRE', PAGE, 'TARGET')),
        'split-run-break': para(run('PRE') + text_run(PAGE) + run('TARGET')),
        'no-break': para(run('PRETARGET')),
    }
    probes = {}
    for width in (1980, 2070, 2160, 4140):
        for name, content in break_cases.items():
            inner = table([(first, side), (content, side)], cell_width=width)
            probes[f'{width}-{name}'] = head + table([(first, side), (inner + first, side)]) + tail
    return probes


def generate():
    for mode in MODES:
        for name, body in cases().items():
            edge.write(f'nested-table-width-page-break-{name}-compat{mode}', body, mode,
                       section_properties=PAGE_GEOMETRY, paragraph_styles=STYLES)


if __name__ == '__main__':
    generate()
