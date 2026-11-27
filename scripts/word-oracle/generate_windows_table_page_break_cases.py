"""Generate DOCX probes for Windows Word's page breaks in table cells."""

import generate_windows_edge_cases as edge
from generate_windows_page_break_cases import (
    BREAK_BEFORE, MODES, PAGE, PAGE_GEOMETRY, SOFT, para, run, text_run,
)
from generate_windows_styled_page_break_cases import COLUMN, COLUMNS, NO_BREAK, style, styled

PREFIX = 'word-windows-edge-table-page-break-'
STYLES = style('RowBreak', 'Normal', BREAK_BEFORE) + style('RowBreakChild', 'RowBreak')
TABLE_STYLE = (
    '<w:style w:type="table" w:default="1" w:styleId="TableNormal">'
    '<w:name w:val="Normal Table"/></w:style>'
)
ZERO_MARGINS = (
    '<w:tblCellMar><w:left w:w="0" w:type="dxa"/><w:right w:w="0" w:type="dxa"/></w:tblCellMar>'
)


def table(rows, *, headers=0, cell_width=4140, style_id=None, indent=None, properties=''):
    column = f'<w:gridCol w:w="{cell_width}"/>'
    parts = [
        '<w:tbl><w:tblPr>',
        '' if style_id is None else f'<w:tblStyle w:val="{style_id}"/>',
        f'<w:tblW w:w="{cell_width * 2}" w:type="dxa"/>',
        '' if indent is None else f'<w:tblInd w:w="{indent}" w:type="dxa"/>',
        '<w:tblLayout w:type="fixed"/>', properties,
        '</w:tblPr><w:tblGrid>', column, column, '</w:tblGrid>',
    ]
    for index, row in enumerate(rows):
        parts.append('<w:tr>')
        if index < headers:
            parts.append('<w:trPr><w:tblHeader/></w:trPr>')
        for content in row:
            parts.append(
                f'<w:tc><w:tcPr><w:tcW w:w="{cell_width}" w:type="dxa"/></w:tcPr>'
                f'{content}</w:tc>'
            )
        parts.append('</w:tr>')
    parts.append('</w:tbl>')
    return ''.join(parts)


def cases():
    head = para(run('HEAD'))
    tail = para(run('TAIL'))
    side = para(run('SIDE'))
    first = para(run('FIRST'))
    row = (first, side)
    second_row = (para(run('SECOND')), side)
    target = run('TARGET')
    forced = para(target, BREAK_BEFORE)
    forced_row = (forced, side)
    inline_cases = {
        'inline-same-run': para(text_run('PRE', PAGE, 'TARGET')),
        'inline-split-run': para(run('PRE') + text_run(PAGE) + target),
        'inline-leading': para(text_run(PAGE, 'TARGET')),
        'inline-terminal': para(text_run('PRE', PAGE)) + para(target),
        'inline-break-only': para(text_run(PAGE)) + para(target),
        'inline-soft-break-after': para(text_run('PRE', PAGE, SOFT, 'TARGET')),
        'inline-page-after-soft-break': para(text_run('PRE', SOFT, PAGE, 'TARGET')),
        'inline-column-break': para(text_run('PRE', COLUMN, 'TARGET')),
        'inline-no-break': para(run('PRETARGET')),
        'inline-large-break-run': para(
            run('PRE') + '<w:r><w:rPr><w:sz w:val="72"/></w:rPr>' + PAGE + '</w:r>' + target,
        ),
    }
    bodies = {
        name: head + table([row, (content, side)])
        for name, content in inline_cases.items()
    }
    bodies.update({
        'first-row-after-body': head + table([forced_row, row]),
        'first-row-document-start': table([forced_row, row]),
        'first-cell-direct': head + table([row, forced_row]),
        'first-cell-inherited': head + table([row, (styled('RowBreakChild', target), side)]),
        'first-cell-direct-off': head + table([row, (styled('RowBreakChild', target, NO_BREAK), side)]),
        'first-cell-empty-with-break': head + table([
            row, (para('', BREAK_BEFORE) + para(target), side),
        ], properties=ZERO_MARGINS),
        'later-paragraph-first-cell': head + table([row, (first + forced, side)]),
        'later-paragraph-after-empty': head + table([row, (para('') + forced, side)]),
        'second-cell-direct': head + table([row, (side, forced)]),
        'second-cell-later-paragraph': head + table([row, (side, first + forced)]),
        'consecutive-rows': head + table([row, forced_row, forced_row]),
        'first-cell-bidi-visual': head + table(
            [row, forced_row], properties=ZERO_MARGINS,
        ).replace('<w:tblPr>', '<w:tblPr><w:bidiVisual/>'),
        'repeated-header': head + table([row, forced_row], headers=1),
        'repeated-header-at-document-start': table([row, forced_row], headers=1),
        'repeated-header-later-body-row': head + table([row, second_row, forced_row], headers=1),
        'repeated-two-headers': head + table([row, second_row, forced_row], headers=2),
        'first-header-has-break': head + table([forced_row, row], headers=1),
        'second-header-has-break': head + table([row, forced_row, second_row], headers=2),
        'repeated-header-consecutive-rows': head + table([row, forced_row, forced_row], headers=1),
        'nested-first-cell': head + table([
            row, (table([row, forced_row], cell_width=1500) + first, side),
        ]),
        'explicit-zero-indent': head + table([row, forced_row], indent=0),
        'explicit-zero-margins': head + table([row, forced_row], properties=ZERO_MARGINS),
        'explicit-top-margin-only': head + table(
            [row, forced_row], properties='<w:tblCellMar><w:top w:w="40" w:type="dxa"/></w:tblCellMar>',
        ),
        'inline-same-run-autofit': head + table(
            [row, (inline_cases['inline-same-run'], side)], properties=ZERO_MARGINS,
        ).replace('w:type="fixed"', 'w:type="autofit"'),
    })
    matrix = {name: (body + tail, PAGE_GEOMETRY, STYLES) for name, body in bodies.items()}
    matrix.update({
        'first-cell-in-columns': (
            head + table([row, forced_row], cell_width=1650) + tail,
            COLUMNS, STYLES,
        ),
        'first-row-at-second-column-top': (
            para(text_run('HEAD', COLUMN))
            + table([forced_row, row], cell_width=1650, properties=ZERO_MARGINS) + tail,
            COLUMNS, STYLES,
        ),
        'explicit-default-table-style': (
            head + table([row, forced_row]) + tail, PAGE_GEOMETRY, STYLES + TABLE_STYLE,
        ),
        'explicit-table-style': (
            head + table([row, forced_row], style_id='TableNormal') + tail,
            PAGE_GEOMETRY, STYLES + TABLE_STYLE,
        ),
    })
    return matrix


def generate():
    probes = cases()
    for mode in MODES:
        for name, (body, geometry, styles) in probes.items():
            edge.write(
                f'table-page-break-{name}-compat{mode}', body, mode,
                section_properties=geometry, paragraph_styles=styles,
            )


if __name__ == '__main__':
    generate()
