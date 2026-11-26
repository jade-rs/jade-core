"""Generate floating tables with both row overflow and authored page breaks."""

from generate_windows_floating_table_break_cases import floating
from generate_windows_table_page_break_cases import (
    BREAK_BEFORE, MODES, PAGE_GEOMETRY, SOFT, STYLES, edge, para, run, text_run,
)

PREFIX = 'word-windows-edge-floating-table-continuation-'


def exact_height(source, row_index, height_pt):
    rows = source.split('<w:tr>')
    rows[row_index + 1] = (
        f'<w:trPr><w:trHeight w:val="{height_pt * 20}" w:hRule="exact"/></w:trPr>'
        + rows[row_index + 1]
    )
    return '<w:tr>'.join(rows)


def cases():
    head, tail = para(run('HEAD')), para(run('TAIL'))
    side = para(run('SIDE'))
    first = (para(run('FIRST')), side)
    forced = (para(run('TARGET'), BREAK_BEFORE), side)
    next_forced = (para(run('NEXT'), BREAK_BEFORE), side)

    def long_row(lines, paragraph_properties=''):
        text = text_run(*(['ROW', SOFT] * (lines - 1) + ['LAST']))
        return (para(text, paragraph_properties), side)

    long = long_row(52)
    long_forced = long_row(52, BREAK_BEFORE)
    tables = {
        'natural-then-explicit': floating([long, forced]),
        'explicit-then-natural': floating([first, long_forced]),
        'natural-and-two-explicit': floating([long, forced, next_forced]),
        'first-and-natural': floating([forced, long]),
        'natural-only': floating([first, long]),
        'natural-only-first-row': floating([long, first]),
        'header-before-natural': floating([first, long, forced], headers=1),
        'header-and-explicit-then-natural': floating([first, long_forced], headers=1),
        'header-and-natural-only': floating([first, long], headers=1),
        'margin-natural-then-explicit': floating([long, forced], 'margin'),
        'page-natural-then-explicit': floating([long, forced], 'page'),
        'legacy-first-row-fits': exact_height(exact_height(floating([first, forced]), 1, 340), 0, 300),
        'legacy-first-row-crosses-margin': exact_height(
            exact_height(floating([first, forced]), 1, 340), 0, 360,
        ),
        'legacy-continued-row-crosses-margin': exact_height(floating([long, forced]), 1, 56),
        'legacy-continued-row-crosses-page': exact_height(floating([long, forced]), 1, 300),
    }
    tables.update({
        f'natural-row-{lines}-lines': floating([long_row(lines), forced])
        for lines in (46, 47, 48, 98)
    })
    return {name: head + grid + tail for name, grid in tables.items()}


def generate():
    probes = cases()
    for mode in MODES:
        for name, body in probes.items():
            edge.write(
                f'floating-table-continuation-{name}-compat{mode}', body, mode,
                section_properties=PAGE_GEOMETRY, paragraph_styles=STYLES,
            )


if __name__ == '__main__':
    generate()
