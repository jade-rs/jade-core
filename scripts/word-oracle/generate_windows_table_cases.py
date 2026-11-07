"""Deterministic modern Word table alignment and border probes."""
import generate_windows_edge_cases as m


def alignment_sweep(compatibility_mode=15, variant=None):
    body = []
    for align in ['left', 'center', 'right']:
        body.append(m.para(m.run('Before ' + align)))
        rows = []
        for r, valign in enumerate(['top', 'center', 'bottom']):
            cells = []
            for c, halign in enumerate(['left', 'center', 'right']):
                content = m.para(m.run(f'R{r+1}C{c+1} alpha'), f'<w:jc w:val="{halign}"/>') + m.para(m.run('Beta 123'), f'<w:jc w:val="{halign}"/>')
                cells.append(f'<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/><w:vAlign w:val="{valign}"/></w:tcPr>{content}</w:tc>')
            rows.append('<w:tr><w:trPr><w:trHeight w:val="1200" w:hRule="exact"/></w:trPr>' + ''.join(cells) + '</w:tr>')
        border = ''.join(f'<w:{side} w:val="single" w:sz="4" w:color="000000"/>' for side in ['top', 'left', 'bottom', 'right', 'insideH', 'insideV'])
        margin = ''.join(f'<w:{side} w:w="{value}" w:type="dxa"/>' for side, value in [('top', 80), ('left', 100), ('bottom', 80), ('right', 100)])
        extra = {'bidi': '<w:bidiVisual/>', 'spacing': '<w:tblCellSpacing w:w="120" w:type="dxa"/>'}.get(variant, '')
        body.append(f'<w:tbl><w:tblPr>{extra}<w:tblW w:w="6000" w:type="dxa"/><w:jc w:val="{align}"/><w:tblLayout w:type="fixed"/><w:tblBorders>{border}</w:tblBorders><w:tblCellMar>{margin}</w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>' + ''.join(rows) + '</w:tbl>')
        body.append(m.para(m.run('After ' + align)))
    suffix = '' if compatibility_mode == 15 else f'-compat{compatibility_mode}'
    name = 'table-alignment' + suffix if variant is None else f'table-{variant}-compat{compatibility_mode}'
    m.write(name, ''.join(body), compatibility_mode)


def border_sweep(compatibility_mode=15, variant=None):
    body = []
    for padding in [0, 80]:
        for size in [0, 4, 8, 16, 24]:
            body.append(m.para(m.run(f'Before P{padding} S{size}')))
            rows = []
            for r in range(2):
                rows.append('<w:tr><w:trPr><w:trHeight w:val="600" w:hRule="exact"/></w:trPr>' + ''.join(f'<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr>' + m.para(m.run(f'R{r+1}C{c+1}')) + '</w:tc>' for c in range(3)) + '</w:tr>')
            border = ''.join(f'<w:{side} w:val="' + ('nil' if size == 0 else 'single') + f'" w:sz="{size}" w:color="000000"/>' for side in ['top', 'left', 'bottom', 'right', 'insideH', 'insideV'])
            horizontal_padding = 0 if variant == 'zero-horizontal' else 100
            margin = ''.join(f'<w:{side} w:w="{value}" w:type="dxa"/>' for side, value in [('top', padding), ('left', horizontal_padding), ('bottom', padding), ('right', horizontal_padding)])
            extra = '<w:tblCellSpacing w:w="120" w:type="dxa"/>' if variant == 'spacing' else ''
            body.append(f'<w:tbl><w:tblPr>{extra}<w:tblW w:w="6000" w:type="dxa"/><w:tblLayout w:type="fixed"/><w:tblBorders>{border}</w:tblBorders><w:tblCellMar>{margin}</w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>' + ''.join(rows) + '</w:tbl>')
            body.append(m.para(m.run(f'After P{padding} S{size}')))
    suffix = '' if compatibility_mode == 15 else f'-compat{compatibility_mode}'
    name = 'table-borders' + suffix if variant is None else f'table-{variant}-borders-compat{compatibility_mode}'
    m.write(name, ''.join(body), compatibility_mode)


def legacy_indent_sweep(compatibility_mode, zero_padding=False):
    body = []
    for align in ['left', 'center', 'right']:
        for indent in [None, 0, 360, -180]:
            label = f'{align} I{indent}'
            body.append(m.para(m.run('Before ' + label)))
            left, right = (0, 60) if zero_padding else (100, 160)
            margins = ''.join(f'<w:{side} w:w="{value}" w:type="dxa"/>' for side, value in [('top', 0), ('left', left), ('bottom', 0), ('right', right)])
            borders = ''.join(f'<w:{side} w:val="single" w:sz="4" w:color="000000"/>' for side in ['top', 'left', 'bottom', 'right', 'insideH', 'insideV'])
            indent_xml = '' if indent is None else f'<w:tblInd w:w="{indent}" w:type="dxa"/>'
            cells = ''.join(f'<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr>' + m.para(m.run(f'C{col+1} alpha')) + '</w:tc>' for col in range(3))
            body.append(f'<w:tbl><w:tblPr><w:tblW w:w="6000" w:type="dxa"/><w:jc w:val="{align}"/>{indent_xml}<w:tblLayout w:type="fixed"/><w:tblBorders>{borders}</w:tblBorders><w:tblCellMar>{margins}</w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:trPr><w:trHeight w:val="600" w:hRule="exact"/></w:trPr>{cells}</w:tr></w:tbl>')
            body.append(m.para(m.run('After ' + label)))
    kind = 'zero-padding' if zero_padding else 'indents'
    m.write(f'table-{kind}-compat{compatibility_mode}', ''.join(body), compatibility_mode)


def spacing_continuation_sweep(compatibility_mode):
    body = []
    for spacing in [40, 120]:
        for size in [0, 4, 8, 16, 24]:
            if body:
                body.append(m.para('<w:r><w:br w:type="page"/></w:r>'))
            body.append(m.para(m.run(f'Before G{spacing} S{size}')))
            border = ''.join(f'<w:{side} w:val="' + ('nil' if size == 0 else 'single') + f'" w:sz="{size}" w:color="000000"/>' for side in ['top', 'left', 'bottom', 'right', 'insideH', 'insideV'])
            margins = ''.join(f'<w:{side} w:w="80" w:type="dxa"/>' for side in ['top', 'left', 'bottom', 'right'])
            rows = []
            for r in range(3):
                cells = []
                for c, valign in enumerate(['top', 'center', 'bottom']):
                    cells.append(f'<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/><w:vAlign w:val="{valign}"/></w:tcPr>' + m.para(m.run(f'R{r+1}C{c+1} alpha')) + m.para(m.run('Beta 123')) + '</w:tc>')
                rows.append('<w:tr><w:trPr><w:trHeight w:val="4800" w:hRule="exact"/></w:trPr>' + ''.join(cells) + '</w:tr>')
            body.append(f'<w:tbl><w:tblPr><w:tblW w:w="6000" w:type="dxa"/><w:tblCellSpacing w:w="{spacing}" w:type="dxa"/><w:tblLayout w:type="fixed"/><w:tblBorders>{border}</w:tblBorders><w:tblCellMar>{margins}</w:tblCellMar></w:tblPr><w:tblGrid>' + '<w:gridCol w:w="2000"/>' * 3 + '</w:tblGrid>' + ''.join(rows) + '</w:tbl>')
            body.append(m.para(m.run(f'After G{spacing} S{size}')))
    m.write(f'table-spacing-continuation-compat{compatibility_mode}', ''.join(body), compatibility_mode)


if __name__ == '__main__':
    for mode in [12, 14, 15]:
        alignment_sweep(mode)
        border_sweep(mode)
        alignment_sweep(mode, 'bidi')
        alignment_sweep(mode, 'spacing')
        border_sweep(mode, 'spacing')
        spacing_continuation_sweep(mode)
        if mode < 15:
            legacy_indent_sweep(mode)
            legacy_indent_sweep(mode, zero_padding=True)
            border_sweep(mode, 'zero-horizontal')
