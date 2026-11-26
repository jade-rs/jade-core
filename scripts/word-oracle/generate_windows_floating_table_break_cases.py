"""Generate Windows Word probes for page breaks in floating tables."""

from generate_windows_styled_page_break_cases import NO_BREAK, styled
from generate_windows_table_page_break_cases import (
    BREAK_BEFORE, MODES, PAGE, PAGE_GEOMETRY, STYLES, ZERO_MARGINS, edge, para, run, table,
    text_run,
)

PREFIX = 'word-windows-edge-floating-table-break-'


def floating(rows, anchor='text', *, headers=0, right_distance=0):
    right = '' if right_distance is None else f' w:rightFromText="{right_distance}"'
    positioning = (
        f'<w:tblpPr w:vertAnchor="{anchor}" w:horzAnchor="margin"'
        f' w:tblpX="0" w:tblpY="0" w:leftFromText="0"{right}/>'
    )
    return table(rows, headers=headers, properties=ZERO_MARGINS).replace(
        '<w:tblPr>', '<w:tblPr>' + positioning, 1,
    )


def cases():
    head, tail = para(run('HEAD')), para(run('TAIL'))
    side = para(run('SIDE'))
    first = (para(run('FIRST')), side)
    second = (para(run('SECOND')), side)
    target = run('TARGET')
    forced = (para(target, BREAK_BEFORE), side)
    next_forced = (para(run('NEXT'), BREAK_BEFORE), side)
    inherited = (styled('RowBreakChild', target), side)
    disabled = (styled('RowBreakChild', target, NO_BREAK), side)
    same_run = (para(text_run('PRE', PAGE, 'TARGET')), side)
    split_run = (para(run('PRE') + text_run(PAGE) + target), side)
    return {
        'text-no-break': head + floating([first, second]) + tail,
        'text-first-row': head + floating([forced, second]) + tail,
        'text-second-row': head + floating([first, forced]) + tail,
        'text-third-row': head + floating([first, second, forced]) + tail,
        'text-no-following-paragraph': head + floating([first, forced]),
        'text-no-preceding-paragraph': floating([first, forced]) + tail,
        'text-repeated-header': head + floating([first, forced], headers=1) + tail,
        'margin-first-row': head + floating([forced, second], 'margin') + tail,
        'margin-second-row': head + floating([first, forced], 'margin') + tail,
        'page-first-row': head + floating([forced, second], 'page') + tail,
        'page-second-row': head + floating([first, forced], 'page') + tail,
        'text-first-and-second': head + floating([forced, next_forced]) + tail,
        'text-second-and-third': head + floating([first, forced, next_forced]) + tail,
        'text-second-and-fourth': head + floating([first, forced, second, next_forced]) + tail,
        'margin-second-and-third': head + floating([first, forced, next_forced], 'margin') + tail,
        'margin-two-breaks-at-start': floating([first, forced, next_forced], 'margin') + tail,
        'text-header-and-two-breaks': head + floating([first, forced, next_forced], headers=1) + tail,
        'text-inherited-first-row': head + floating([inherited, second]) + tail,
        'text-inherited-second-row': head + floating([first, inherited]) + tail,
        'text-inherited-disabled': head + floating([first, disabled]) + tail,
        'text-inline-same-run': head + floating([first, same_run]) + tail,
        'text-inline-split-run': head + floating([first, split_run]) + tail,
        'text-first-row-small-wrap-distance': head + floating([forced, second], right_distance=16) + tail,
        'text-first-row-wrap-distance': head + floating([forced, second], right_distance=40) + tail,
        'text-second-row-wrap-distance': head + floating([first, forced], right_distance=100) + tail,
        'text-no-break-wrap-distance': head + floating([first, second], right_distance=40) + tail,
        'text-first-row-omitted-wrap-distance': head + floating([forced, second], right_distance=None) + tail,
    }


def generate():
    probes = cases()
    for mode in MODES:
        for name, body in probes.items():
            edge.write(
                f'floating-table-break-{name}-compat{mode}', body, mode,
                section_properties=PAGE_GEOMETRY, paragraph_styles=STYLES,
            )


if __name__ == '__main__':
    generate()
