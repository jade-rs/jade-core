"""Generate Word probes for preferred widths inside nested tables."""

import re

from generate_windows_nested_table_alignment_cases import RIGHT
from generate_windows_nested_table_overflow_cases import document, text, with_row_height
from generate_windows_nested_table_page_break_cases import nested_break_cases
from generate_windows_table_page_break_cases import MODES, PAGE_GEOMETRY, STYLES, edge, table

PREFIX = 'word-windows-edge-nested-preferred-'
AUTO = (0, 'auto')
PERCENT = (5000, 'pct')
OVER = (7500, 'pct')
ABSOLUTE = (4800, 'dxa')
MARKER = edge.para(edge.run('RIGHTEDGE'), RIGHT)


def body(layout, preferred, grid, cells, content, *, properties='', exact=False):
    inner = table([(text('FIRST'), text('INNER')), (content, MARKER)],
                  cell_width=1500, properties=properties)
    frame = '' if preferred is None else f'<w:tblW w:w="{preferred[0]}" w:type="{preferred[1]}"/>'
    inner = inner.replace('<w:tblW w:w="3000" w:type="dxa"/>', frame, 1)
    inner = inner.replace('w:tblLayout w:type="fixed"', f'w:tblLayout w:type="{layout}"', 1)
    saved_grid = ''
    if grid:
        saved_grid = '<w:tblGrid>' + ''.join(f'<w:gridCol w:w="{width}"/>' for width in grid) + '</w:tblGrid>'
    inner = re.sub(r'<w:tblGrid>.*?</w:tblGrid>', saved_grid, inner)
    preferred_cells = iter(cells)

    def cell_width(_):
        size = next(preferred_cells)
        value, unit = (size, 'dxa') if isinstance(size, int) else size
        return f'<w:tcW w:w="{value}" w:type="{unit}"/>'

    inner = re.sub(r'<w:tcW w:w="1500" w:type="dxa"/>', cell_width, inner)
    outer = table([(text('OUTER'), inner + text('CELLTAIL'))])
    return document(with_row_height(outer, 'exact', 1200) if exact else outer)


def cases():
    probes = {}
    breaks = nested_break_cases()
    ratio_controls = {
        'cell-twice': ((1500, 1500), (1500, 3000) * 2, PERCENT),
        'grid-twice': ((1500, 3000), (1500,) * 4, PERCENT),
        'second-row': ((1500, 1500), (1500, 1500, 3000, 1500), PERCENT),
        'conflict': ((1500, 1500), (1500, 3000, 3000, 1500), PERCENT),
        'grid-twice-auto': ((1500, 3000), (1500,) * 4, AUTO),
        'cell-twice-auto': ((1500, 1500), (1500, 3000) * 2, AUTO),
        'cell-percent': ((1500, 1500), ((1250, 'pct'), (3750, 'pct')) * 2, PERCENT),
        'cell-auto': ((1500, 3000), (AUTO,) * 4, PERCENT),
        'wide-auto': ((2400, 2400), (2400,) * 4, AUTO),
        'wide-omitted': ((2400, 2400), (2400,) * 4, None),
    }
    mixed_controls = {
        'left-only': ((1500, 3000), (1500, AUTO) * 2, PERCENT),
        'right-only': ((1500, 3000), (AUTO, 1500) * 2, PERCENT),
        'left-only-auto': ((1500, 3000), (1500, AUTO) * 2, AUTO),
        'mixed-percent': ((1500, 1500), (1500, (3750, 'pct')) * 2, PERCENT),
        'mixed-percent-auto': ((1500, 1500), (1500, (3750, 'pct')) * 2, AUTO),
        'no-grid': ((), (1500, 3000) * 2, PERCENT),
        'no-grid-auto': ((), (1500, 3000) * 2, AUTO),
        'spaced-percent': ((1500, 1500), (1500,) * 4, PERCENT),
        'spaced-dxa': ((1500, 1500), (1500,) * 4, ABSOLUTE),
        'aligned-right': ((1500, 1500), (1500,) * 4, OVER),
        'aligned-center': ((1500, 1500), (1500,) * 4, OVER),
        'exact-over': ((1500, 1500), (1500,) * 4, OVER),
    }
    for layout in ('fixed', 'autofit'):
        for name, preferred in (('auto', AUTO), ('half', (2500, 'pct')), ('full', PERCENT),
                                ('over', OVER), ('absolute', ABSOLUTE)):
            # The wider automatic case below keeps its last glyph on the page.
            for width in (1500,) if name == 'auto' else (1500, 4140):
                for kind in ('same-run-break', 'paragraph-break-before'):
                    probes[f'{layout}-{name}-{width}-{kind}'] = body(
                        layout, preferred, (width, width), (width,) * 4, breaks[kind],
                    )
        for name, (grid, cells, preferred) in ratio_controls.items():
            probes[f'ratios-{layout}-{name}'] = body(layout, preferred, grid, cells,
                                                     breaks['same-run-break'])
        for name, (grid, cells, preferred) in mixed_controls.items():
            properties = ''
            if name.startswith('spaced-'):
                properties = '<w:tblCellSpacing w:w="120" w:type="dxa"/>'
            if name.startswith('aligned-'):
                properties = f'<w:jc w:val="{name.split("-")[1]}"/>'
            probes[f'mixed-{layout}-{name}'] = body(
                layout, preferred, grid, cells, breaks['same-run-break'], properties=properties,
                exact=name == 'exact-over',
            )
    return probes


def generate():
    probes = cases()
    for mode in MODES:
        for name, content in probes.items():
            edge.write(f'nested-preferred-{name}-compat{mode}', content, mode,
                       section_properties=PAGE_GEOMETRY, paragraph_styles=STYLES)


if __name__ == '__main__':
    generate()
