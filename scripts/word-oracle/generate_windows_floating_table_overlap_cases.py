"""Generate Word probes for neighboring floating tables and their wrap boundaries."""

from generate_windows_floating_table_wrap_cases import SIX_LINES, TAIL
from generate_windows_table_page_break_cases import (
    MODES, PAGE_GEOMETRY, STYLES, edge, para, run, table,
)

PREFIX = 'word-windows-edge-floating-table-overlap-'


def placed(token, y, *, align=None, horizontal='margin', vertical='margin',
           cell_width=4140, left_gap=0, right_gap=0, inset=0, rules=(), forbid=False):
    x = 'w:tblpX="0"' if align is None else f'w:tblpXSpec="{align}"'
    properties = (
        f'<w:tblpPr w:horzAnchor="{horizontal}" w:vertAnchor="{vertical}" {x}'
        f' w:tblpY="{round(20 * y)}" w:leftFromText="{left_gap}"'
        f' w:rightFromText="{right_gap}"/>'
    )
    if forbid:
        properties += '<w:tblOverlap w:val="never"/>'
    if rules:
        properties += '<w:tblBorders>' + ''.join(
            f'<w:{side} w:val="single" w:sz="{width}" w:color="000000"/>'
            for side, width in rules
        ) + '</w:tblBorders>'
    properties += (
        f'<w:tblCellMar><w:left w:w="{inset}" w:type="dxa"/>'
        f'<w:right w:w="{inset}" w:type="dxa"/></w:tblCellMar>'
    )
    rows = [(para(run(f'{token}{index}')), para(run('SIDE'))) for index in (1, 2, 3)]
    return table(rows, cell_width=cell_width, properties=properties)


def cases():
    mid = para(run('MID'))
    tail = ''.join(para(run(word)) for word in ('NEXT', 'OTHER', 'AFTER')) + TAIL
    first = placed('ROW', 14)
    second = placed('TWO', 24)
    first_forbids = placed('ROW', 14, forbid=True)
    second_forbids = placed('TWO', 24, forbid=True)
    pairs = {
        'both-allow': (first, second),
        'first-forbids': (first_forbids, second),
        'second-forbids': (first, second_forbids),
        'both-forbid': (first_forbids, second_forbids),
        'opposite-sides': (first, placed('TWO', 24, align='right')),
        'second-relative-above': (first, placed('TWO', -25, vertical='text')),
    }
    for label, y in [('before', 52.4), ('touching', 55.4), ('after', 58.4)]:
        pairs[f'authored-{label}'] = (first, placed('TWO', y))
    probes = {name: SIX_LINES + one + mid + two + tail for name, (one, two) in pairs.items()}
    for gap in (0, 5, 10, 15, 20, 40, 100):
        probes[f'displaced-right-gap-{gap}'] = (
            SIX_LINES + first + mid + placed('TWO', 24, right_gap=gap, forbid=True) + tail
        )
        right = '<w:jc w:val="right"/>'
        before = ''.join(para(run(word), right) for word in ('ONE', 'TWO', 'THREE', 'FOUR', 'FIVE', 'SIX'))
        after = ''.join(para(run(word), right) for word in ('NEXT', 'OTHER', 'TAIL'))
        probes[f'displaced-left-gap-{gap}'] = (
            before + placed('ROW', 14, align='right') + para(run('MID'), right)
            + placed('TWO', 24, align='right', left_gap=gap, forbid=True) + after
        )
    for name, grid in [
        ('right-zero', placed('TWO', 24, align='right')),
        ('right-narrow', placed('TWO', 24, align='right', cell_width=3000)),
        ('right-page', placed('TWO', 24, align='right', horizontal='page')),
        ('right-inset', placed('TWO', 24, align='right', inset=100)),
        ('right-border', placed('TWO', 24, align='right', rules=tuple(
            (side, 8) for side in ('top', 'left', 'bottom', 'right', 'insideH', 'insideV')
        ))),
        ('right-right-rule', placed('TWO', 24, align='right', rules=(('right', 8),))),
        ('right-asymmetric-rules', placed('TWO', 24, align='right', rules=(('left', 4), ('right', 16)))),
    ]:
        probes[name] = SIX_LINES + grid + mid + tail
    return probes


def generate():
    for mode in MODES:
        for name, body in cases().items():
            edge.write(
                f'floating-table-overlap-{name}-compat{mode}', body, mode,
                section_properties=PAGE_GEOMETRY, paragraph_styles=STYLES,
            )


if __name__ == '__main__':
    generate()
