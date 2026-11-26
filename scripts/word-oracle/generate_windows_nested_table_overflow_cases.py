"""Generate Word probes for nested tables in growing and exact-height rows."""

from generate_windows_nested_table_page_break_cases import nested_break_cases
from generate_windows_table_page_break_cases import (
    BREAK_BEFORE, MODES, PAGE_GEOMETRY, STYLES, ZERO_MARGINS, edge, para, run, table,
)

PREFIX = 'word-windows-edge-nested-table-overflow-'


def text(word):
    return para(run(word))


def document(grid):
    return text('HEAD') + grid + text('TAIL')


def nested(content, width=4140):
    rows = [(text('FIRST'), text('INNER')), (content, text('TARGETSIDE'))]
    return table(rows, cell_width=width) + text('CELLTAIL')


def spaced_properties(spacing):
    return ZERO_MARGINS + f'<w:tblCellSpacing w:w="{spacing}" w:type="dxa"/>'


def with_row_height(grid, rule, height):
    properties = f'<w:trPr><w:trHeight w:val="{height}" w:hRule="{rule}"/></w:trPr>'
    return grid.replace('<w:tr>', '<w:tr>' + properties, 1)


def repeating_rows(inner, sibling):
    return [
        (inner, sibling), (text('BODY'), text('BODYRIGHT')),
        (para(run('NEXT'), BREAK_BEFORE), text('NEXTRIGHT')),
    ]


def context_cases():
    probes = {}
    sibling = text('OUTER')
    for width in (2070, 4140):
        for name, content in nested_break_cases().items():
            inner = nested(content, width)
            grids = {
                'second-cell': table([(sibling, inner)]),
                'rtl-parent': table([(inner, sibling)], properties='<w:bidiVisual/>'),
                'spaced-parent': table([(inner, sibling)], properties=spaced_properties(120)),
                'repeat-header': table(repeating_rows(inner, sibling), headers=1),
            }
            for context, grid in grids.items():
                probes[f'{context}-{width}-{name}'] = document(grid)
    return probes


def repeated_row_cases():
    probes = {}
    breaks = nested_break_cases()
    for width in (2070, 4140):
        for name in ('same-run-break', 'paragraph-break-before'):
            rows = repeating_rows(nested(breaks[name], width), text('OUTER'))
            spaced = table(rows, headers=1, properties=spaced_properties(120))
            exact = spaced.replace(
                '<w:trPr><w:tblHeader/>',
                '<w:trPr><w:trHeight w:val="500" w:hRule="exact"/><w:tblHeader/>',
                1,
            )
            probes[f'repeated-spaced-{width}-{name}'] = document(spaced)
            probes[f'repeated-exact-spaced-{width}-{name}'] = document(exact)
    return probes


def row_height_cases():
    probes = {}
    sibling = text('OUTER')
    for width in (2070, 4140):
        for name, content in nested_break_cases().items():
            grid = table([(nested(content, width), sibling)])
            probes[f'short-exact-row-{width}-{name}'] = document(with_row_height(grid, 'exact', 500))
    for spacing in (0, 120):
        grid = table([(nested(text('TARGET')), sibling)], properties=spaced_properties(spacing))
        for height in (400, 500, 550, 650, 800, 1200):
            probes[f'height-{spacing}-exact-{height}'] = document(with_row_height(grid, 'exact', height))
        probes[f'height-{spacing}-minimum-500'] = document(with_row_height(grid, 'atLeast', 500))
    return probes | repeated_row_cases()


def generate():
    cases = context_cases() | row_height_cases()
    for mode in MODES:
        for name, body in cases.items():
            edge.write(f'nested-table-overflow-{name}-compat{mode}', body, mode,
                       section_properties=PAGE_GEOMETRY, paragraph_styles=STYLES)


if __name__ == '__main__':
    generate()
