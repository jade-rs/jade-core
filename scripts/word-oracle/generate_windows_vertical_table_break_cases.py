"""Generate Windows Word witnesses for page breaks in vertical table text."""

from generate_windows_table_page_break_cases import (
    MODES, PAGE, PAGE_GEOMETRY, SOFT, STYLES, ZERO_MARGINS, edge, para, run, table, text_run,
)

PREFIX = 'word-windows-edge-vertical-table-break-'


def cases():
    head, tail = para(run('HEAD')), para(run('TAIL'))
    side = para(run('SIDE'))
    first = (para(run('FIRST')), side)
    joined = 'PRETARGET'
    page = text_run('PRE', PAGE, 'TARGET')

    def probe(direction, content, peer=side, height_rule=None, vertical_peer=False,
              indent=None):
        target = para(content)
        source = table([first, (target, peer)], indent=indent, properties=ZERO_MARGINS)
        marker = f'<w:textDirection w:val="{direction}"/></w:tcPr>'
        source = source.replace('</w:tcPr>' + target, marker + target, 1)
        if vertical_peer:
            left, right = source.rsplit('</w:tcPr>' + peer, 1)
            source = left + marker + peer + right
        if height_rule:
            left, right = source.rsplit('<w:tr>', 1)
            height = f'<w:trPr><w:trHeight w:val="700" w:hRule="{height_rule}"/></w:trPr>'
            source = left + '<w:tr>' + height + right
        return head + source + tail

    matrix = {}
    for name, direction in [('turned', 'tbRl'), ('upright', 'tbRlV')]:
        matrix.update({
            f'{name}-same-run': probe(direction, page),
            f'{name}-split-run': probe(direction, run('PRE') + text_run(PAGE) + run('TARGET')),
            f'{name}-no-break': probe(direction, run(joined)),
            f'{name}-exact-height': probe(direction, page, height_rule='exact'),
            f'{name}-minimum-height': probe(direction, page, height_rule='atLeast'),
            f'{name}-two-horizontal-lines': probe(
                direction, page, peer=para(text_run('SIDE', SOFT, 'NEXT')),
            ),
            f'{name}-no-horizontal-cell': probe(direction, page, vertical_peer=True),
            f'{name}-large-font': probe(direction, run(joined, '<w:sz w:val="40"/>')),
            f'{name}-short-text': probe(direction, run('PRE')),
        })
    # Fit the last column of the largest face on the physical sheet.
    # This tests every glyph's track instead of losing it to PDF clipping.
    faces = ('Arial', 'Calibri', 'Aptos', 'Times New Roman', 'MS Mincho', 'Yu Gothic')
    for face in faces:
        name = face.lower().replace(' ', '-')
        for size in (24, 40):
            props = (
                f'<w:rPr><w:rFonts w:ascii="{face}" w:hAnsi="{face}"/>'
                f'<w:sz w:val="{size}"/></w:rPr>'
            )
            matrix[f'font-{name}-{size}'] = probe(
                'tbRl', text_run(props, 'PRE', PAGE, 'TARGET'), indent=720,
            )

    japanese = '<w:rPr><w:rFonts w:eastAsia="MS Mincho"/></w:rPr>'
    for name, direction in [('turned', 'tbRl'), ('upright', 'tbRlV'), ('bottom-up', 'btLr')]:
        matrix[f'japanese-{name}'] = probe(direction, text_run(japanese, '改頁', PAGE, '見本'))
        matrix[f'mixed-{name}'] = probe(direction, text_run(japanese, '前PRE', PAGE, '後TEST'))
        matrix[f'extended-latin-{name}'] = probe(direction, text_run('pré', PAGE, 'TÄRGET'))
    return matrix


def generate():
    probes = cases()
    for mode in MODES:
        for name, body in probes.items():
            edge.write(
                f'vertical-table-break-{name}-compat{mode}', body, mode,
                section_properties=PAGE_GEOMETRY, paragraph_styles=STYLES,
            )


if __name__ == '__main__':
    generate()
