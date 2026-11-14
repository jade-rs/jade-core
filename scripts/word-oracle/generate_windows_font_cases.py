"""Deterministic Windows Word font-size, tracking and mixed-script probes."""
import generate_windows_edge_cases as edge


def advance_sweep(family, face, compatibility=15):
    paragraphs = []
    for half_points in range(12, 61):
        font = (f'<w:rFonts w:ascii="{family}" w:hAnsi="{family}" '
                f'w:eastAsia="{family}" w:cs="{family}"/>'
                f'<w:sz w:val="{half_points}"/><w:szCs w:val="{half_points}"/>'
                '<w:lang w:val="en-US" w:eastAsia="ja-JP"/>')
        effects = {'regular': '', 'bold': '<w:b/>', 'italic': '<w:i/>',
                   'bold-italic': '<w:b/><w:i/>'}[face]
        properties = font + effects
        paragraphs.append(edge.para(
            edge.run('Hello World 0123456789', properties),
            '<w:rPr>' + properties + '</w:rPr>'))
    slug = family.lower().replace(' ', '-')
    edge.write(f'{slug}-latin-advances-{face}-compat{compatibility}',
               ''.join(paragraphs), compatibility)


def tracking_sweep(family, compatibility):
    paragraphs = []
    for half_points in (19, 20, 21, 22, 23, 24, 28, 32):
        for tracking in (-10, 0, 10, 30, 600):
            for text in ('Hello World', 'ABC漢字DEF'):
                font = (f'<w:rFonts w:ascii="{family}" w:hAnsi="{family}" '
                        f'w:eastAsia="{family}"/>'
                        f'<w:sz w:val="{half_points}"/>'
                        f'<w:spacing w:val="{tracking}"/>')
                paragraphs.append(edge.para(edge.run(text, font),
                                           '<w:rPr>' + font + '</w:rPr>'))
    slug = family.lower().replace(' ', '-')
    edge.write(f'{slug}-tracking-mixed-compat{compatibility}',
               ''.join(paragraphs), compatibility)



def kerning_tracking_sweep(compatibility):
    paragraphs = []
    for half_points in (21, 24, 40):
        for bold in (False, True):
            for tracking in (0, 10, 30, 600):
                for kern in (0, 2):
                    font = ('<w:rFonts w:ascii="Yu Mincho" w:hAnsi="Yu Mincho" '
                            'w:eastAsia="Yu Mincho"/>'
                            f'<w:sz w:val="{half_points}"/>'
                            f'<w:spacing w:val="{tracking}"/>'
                            f'<w:kern w:val="{kern}"/>' + ('<w:b/>' if bold else ''))
                    paragraphs.append(edge.para(edge.run('Hello World', font),
                                               '<w:rPr>' + font + '</w:rPr>'))
    edge.write(f'yu-mincho-kerning-tracking-compat{compatibility}',
               ''.join(paragraphs), compatibility)


def main():
    for family in ('Yu Mincho', 'MS Mincho', 'Arial'):
        for face in ('regular', 'bold', 'italic', 'bold-italic'):
            advance_sweep(family, face)
    for compatibility in (12, 14, 15):
        tracking_sweep('Yu Mincho', compatibility)
        kerning_tracking_sweep(compatibility)


if __name__ == '__main__':
    main()
