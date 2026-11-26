"""Generate Word probes for body text preceding a floating table."""

from generate_windows_floating_table_break_cases import floating
from generate_windows_table_page_break_cases import (
    BREAK_BEFORE, MODES, PAGE, PAGE_GEOMETRY, SOFT, STYLES, edge, para, run, text_run,
)

PREFIX = 'word-windows-edge-floating-table-wrap-'
SIX_LINES = ''.join(para(run(word)) for word in ('ONE', 'TWO', 'THREE', 'FOUR', 'FIVE', 'SIX'))
TAIL = para(run('TAIL'))


def positioned(anchor, vertical_attributes, row_count=4):
    rows = [(para(run(f'ROW{index + 1}')), para(run('SIDE'))) for index in range(row_count)]
    return floating(rows, anchor).replace('w:tblpY="0"', vertical_attributes)


def offset(y_pt):
    return f'w:tblpY="{20 * y_pt}"'


def padded_paragraph(lines):
    return para(text_run(*(['PAD', SOFT] * (lines - 1) + ['END'])))


def cases(mode=None):
    probes = {}
    for anchor in ('margin', 'page', 'text'):
        distances = (0, 42, 72, 86, 120) if anchor == 'page' else (-30, 0, 14, 42, 100)
        for y_pt in distances:
            name = f'{anchor}-above-{abs(y_pt)}' if y_pt < 0 else f'{anchor}-offset-{y_pt}'
            probes[name] = SIX_LINES + positioned(anchor, offset(y_pt)) + TAIL + para(run('PAR'))
    for anchor, y_pt in [('margin', 0), ('page', 72), ('text', -28)]:
        heading = para(text_run('ONE', SOFT, 'TWO', SOFT, 'THREE', SOFT, 'FOUR'))
        body = para(run(('a word for wrapping ' * 20).strip()))
        probes[f'{anchor}-multiple-lines'] = (
            heading + body + positioned(anchor, offset(y_pt)) + TAIL + para(run('PAR'))
        )
    for anchor in ('margin', 'page'):
        for name, position in [
            ('omitted', ''),
            ('one-twip', 'w:tblpY="1"'),
            ('top-alignment', 'w:tblpYSpec="top"'),
        ]:
            probes[f'{anchor}-{name}'] = SIX_LINES + positioned(anchor, position) + TAIL
    for lines in (41, 42, 43, 44, 45, 46, 47):
        for rows in (1, 2, 4):
            probes[f'page-bottom-{lines}-lines-{rows}-rows'] = (
                padded_paragraph(lines) + positioned('margin', offset(68), rows) + TAIL
            )
    for lines in (39, 44, 45):
        if mode == 15 and lines != 39:
            # Word clears the table for a word wider than the available side.
            # These two captured probes are pending that behavior.
            continue
        body = padded_paragraph(lines + 1) + para(run(('WRAPWORD ' * 30).strip()))
        probes[f'page-bottom-rewrap-{lines + 1}-lines'] = (
            body + positioned('margin', offset(68)) + TAIL
        )
    for name, tail in [
        ('no-following-text', ''),
        ('following-break-before', para(run('TAIL'), BREAK_BEFORE)),
        ('following-inline-break', para(text_run(PAGE, 'TAIL'))),
    ]:
        probes[name] = para(run('HEAD')) + positioned('margin', offset(0), 1) + tail
    return probes


def generate():
    for mode in MODES:
        for name, body in cases(mode).items():
            edge.write(
                f'floating-table-wrap-{name}-compat{mode}', body, mode,
                section_properties=PAGE_GEOMETRY, paragraph_styles=STYLES,
            )


if __name__ == '__main__':
    generate()
