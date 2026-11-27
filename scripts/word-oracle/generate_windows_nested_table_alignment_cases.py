"""Generate fixed-width nested table placement and clipping probes."""

from generate_windows_nested_table_overflow_cases import (
    document, repeating_rows, text, with_row_height,
)
from generate_windows_nested_table_page_break_cases import nested_break_cases
from generate_windows_table_page_break_cases import MODES, PAGE_GEOMETRY, STYLES, edge, table

PREFIX = 'word-windows-edge-nested-table-alignment-'
LEFT = '<w:jc w:val="left"/>'
CENTER = '<w:jc w:val="center"/>'
RIGHT = '<w:jc w:val="right"/>'
RTL = '<w:bidiVisual/>'
STYLES_BY_CASE = {
    'left': (None, LEFT),
    'center': (None, CENTER),
    'right': (None, RIGHT),
    'indent-positive': (360, ''),
    'indent-negative': (-360, ''),
    'center-indent': (360, CENTER),
    'right-indent': (360, RIGHT),
    'rtl': (None, RTL),
}


def inner_table(content, width, style, target_side=None):
    indent, properties = style
    target_side = text('TARGETSIDE') if target_side is None else target_side
    rows = [(text('FIRST'), text('INNER')), (content, target_side)]
    return table(rows, cell_width=width, indent=indent, properties=properties) + text('CELLTAIL')


def cases():
    probes = {}
    breaks = nested_break_cases()
    sibling = text('OUTER')
    fitting = ('left', 'center', 'right', 'indent-positive', 'indent-negative', 'center-indent', 'rtl')
    # Use the second outer cell for wide center and right alignment so that
    # the inner table's glyphs stay on the physical page.
    growing = ('left', 'indent-positive', 'indent-negative')
    for width, styles in ((1500, fitting), (4140, growing)):
        for name in styles:
            for kind in ('same-run-break', 'paragraph-break-before'):
                inner = inner_table(breaks[kind], width, STYLES_BY_CASE[name])
                probes[f'{name}-{width}-{kind}'] = document(table([(inner, sibling)]))

    contexts = (
        ('first', 2400, 'same-run-break'),
        ('second', 4140, 'same-run-break'),
        ('exact', 4140, 'same-run-break'),
        ('repeat', 4140, 'paragraph-break-before'),
    )
    for name in ('center', 'indent-positive', 'indent-negative', 'right', 'right-indent', 'rtl'):
        for context, width, kind in contexts:
            inner = inner_table(breaks[kind], width, STYLES_BY_CASE[name])
            cells = (inner, sibling) if context == 'first' else (sibling, inner)
            grid = table([cells])
            if context == 'exact':
                grid = with_row_height(grid, 'exact', 1200)
            if context == 'repeat':
                grid = table(repeating_rows(*cells), headers=1)
            probes[f'{context}-{name}-{width}-{kind}'] = document(grid)

    right_edge = edge.para(edge.run('RIGHTEDGE'), RIGHT)
    for name, width, indent, properties in (
        ('near-fit-indent', 2040, 360, ''),
        ('near-fit-indent-exact', 2040, 360, ''),
        ('rtl-indent', 2400, 360, RTL),
        ('rtl-negative-indent', 2400, -360, RTL),
        ('rtl-center-indent', 2400, 360, RTL + CENTER),
        ('rtl-right-indent', 2400, 360, RTL + RIGHT),
    ):
        inner = inner_table(breaks['same-run-break'], width, (indent, properties), right_edge)
        grid = table([(inner, sibling)])
        if name == 'near-fit-indent-exact':
            grid = with_row_height(grid, 'exact', 1200)
        probes[name] = document(grid)
    return probes


def generate():
    probes = cases()
    for mode in MODES:
        for name, body in probes.items():
            edge.write(f'nested-table-alignment-{name}-compat{mode}', body, mode,
                       section_properties=PAGE_GEOMETRY, paragraph_styles=STYLES)


if __name__ == '__main__':
    generate()
