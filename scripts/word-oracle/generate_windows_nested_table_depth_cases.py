"""Generate Windows Word probes for overflow across nested table cells."""

from generate_windows_nested_table_alignment_cases import CENTER, RIGHT
from generate_windows_nested_table_overflow_cases import (
    document, repeating_rows, text, with_row_height,
)
from generate_windows_nested_table_page_break_cases import nested_break_cases
from generate_windows_table_page_break_cases import MODES, PAGE_GEOMETRY, STYLES, edge, table

PREFIX = 'word-windows-edge-nested-three-level-'
PLACEMENT = {'left': (None, ''), 'center': (None, CENTER), 'right': (None, RIGHT)}
EXACT_HEADER = '<w:trPr><w:trHeight w:val="2000" w:hRule="exact"/><w:tblHeader/>'


def middle_table(content, placement, exact=False):
    indent, properties = placement
    right_edge = edge.para(edge.run('RIGHTEDGE'), RIGHT)
    inner_rows = [(text('FIRST'), text('INNER')), (content, right_edge)]
    inner = table(inner_rows, cell_width=2400, indent=indent, properties=properties)
    middle = table([(text('MIDLEFT'), inner + text('MIDTAIL'))], cell_width=1500)
    if exact:
        middle = with_row_height(middle, 'exact', 1400)
    return middle + text('CELLTAIL')


def cases():
    probes = {}
    breaks = nested_break_cases()
    sibling = text('OUTER')
    for name, position in (PLACEMENT | {'right-indent': (360, RIGHT)}).items():
        for rule in ('growing', 'middle-exact', 'outer-exact'):
            middle = middle_table(breaks['same-run-break'], position, exact=rule == 'middle-exact')
            outer = table([(sibling, middle)])
            if rule == 'outer-exact':
                outer = with_row_height(outer, 'exact', 2000)
            probes[f'{name}-{rule}'] = document(outer)
    for name, position in PLACEMENT.items():
        middle = middle_table(breaks['paragraph-break-before'], position)
        outer = table(repeating_rows(sibling, middle), headers=1)
        probes[f'repeat-{name}-growing'] = document(outer)
        probes[f'repeat-{name}-outer-exact'] = document(
            outer.replace('<w:trPr><w:tblHeader/>', EXACT_HEADER, 1),
        )
    return probes


def generate():
    probes = cases()
    for mode in MODES:
        for name, body in probes.items():
            edge.write(f'nested-three-level-{name}-compat{mode}', body, mode,
                       section_properties=PAGE_GEOMETRY, paragraph_styles=STYLES)


if __name__ == '__main__':
    generate()
