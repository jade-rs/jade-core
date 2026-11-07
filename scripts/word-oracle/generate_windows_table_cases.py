"""Deterministic modern Word table alignment and border probes."""
import generate_windows_edge_cases as m


def alignment_sweep():
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
        body.append(f'<w:tbl><w:tblPr><w:tblW w:w="6000" w:type="dxa"/><w:jc w:val="{align}"/><w:tblLayout w:type="fixed"/><w:tblBorders>{border}</w:tblBorders><w:tblCellMar>{margin}</w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>' + ''.join(rows) + '</w:tbl>')
        body.append(m.para(m.run('After ' + align)))
    m.write('table-alignment', ''.join(body))


def border_sweep():
    body = []
    for padding in [0, 80]:
        for size in [0, 4, 8, 16, 24]:
            body.append(m.para(m.run(f'Before P{padding} S{size}')))
            rows = []
            for r in range(2):
                rows.append('<w:tr><w:trPr><w:trHeight w:val="600" w:hRule="exact"/></w:trPr>' + ''.join(f'<w:tc><w:tcPr><w:tcW w:w="2000" w:type="dxa"/></w:tcPr>' + m.para(m.run(f'R{r+1}C{c+1}')) + '</w:tc>' for c in range(3)) + '</w:tr>')
            border = ''.join(f'<w:{side} w:val="' + ('nil' if size == 0 else 'single') + f'" w:sz="{size}" w:color="000000"/>' for side in ['top', 'left', 'bottom', 'right', 'insideH', 'insideV'])
            margin = ''.join(f'<w:{side} w:w="{value}" w:type="dxa"/>' for side, value in [('top', padding), ('left', 100), ('bottom', padding), ('right', 100)])
            body.append(f'<w:tbl><w:tblPr><w:tblW w:w="6000" w:type="dxa"/><w:tblLayout w:type="fixed"/><w:tblBorders>{border}</w:tblBorders><w:tblCellMar>{margin}</w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>' + ''.join(rows) + '</w:tbl>')
            body.append(m.para(m.run(f'After P{padding} S{size}')))
    m.write('table-borders', ''.join(body))


if __name__ == '__main__':
    alignment_sweep()
    border_sweep()
