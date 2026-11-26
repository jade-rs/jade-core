"""Generate Word probes for fitting a whole word beside a floating table."""

from generate_windows_floating_table_overlap_cases import placed
from generate_windows_floating_table_wrap_cases import TAIL
from generate_windows_table_page_break_cases import MODES, PAGE_GEOMETRY, STYLES, edge, para, run

PREFIX = 'word-windows-edge-floating-table-word-wrap-'
BODY_WIDTH_PT = 468


def cases():
    probes = {}
    following = ''.join(para(run(text)) for text in ('NEXT', 'OVER', 'TAIL'))
    preceding = ''.join(para(run(text)) for text in ('ONE', 'WRAPWORD', 'THREE', 'FOUR', 'FIVE', 'SIX'))
    for align in ('left', 'center', 'right'):
        sides = 2 if align == 'center' else 1
        table = placed('ROW', 0, align=align, vertical='text',
                       cell_width=(BODY_WIDTH_PT - sides * 27) * 10)
        head = para(run('HEAD')) + table
        for label, text in [('wide', 'WRAPWORD'), ('narrow', 'A'), ('mixed', 'A WRAPWORD B')]:
            probes[f'after-anchor-{align}-{label}'] = head + para(run(text)) + following
        for gap in (24, 27, 36, 48):
            table = placed('ROW', 24, align=align, cell_width=(BODY_WIDTH_PT - sides * gap) * 10)
            probes[f'before-anchor-{align}-{gap}'] = preceding + table + TAIL
    return probes


def generate():
    for mode in MODES:
        for name, body in cases().items():
            edge.write(f'floating-table-word-wrap-{name}-compat{mode}', body, mode,
                       section_properties=PAGE_GEOMETRY, paragraph_styles=STYLES)


if __name__ == '__main__':
    generate()
