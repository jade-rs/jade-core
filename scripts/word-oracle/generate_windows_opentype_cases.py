"""Capture the Windows Word typography switch separately from w:kern."""
from zipfile import ZipFile

import generate_windows_edge_cases as edge


def main():
    for compatibility in (12, 14, 15):
        for enabled in (None, 0, 1):
            paragraphs = []
            for ligatures in ('none', 'standard', 'contextual', 'standardContextual'):
                for kern in (0, 2):
                    for tracking in (0, 10):
                        props = ('<w:rFonts w:ascii="Yu Mincho" w:hAnsi="Yu Mincho" '
                                 'w:eastAsia="Yu Mincho"/><w:sz w:val="21"/>'
                                 f'<w:kern w:val="{kern}"/><w:spacing w:val="{tracking}"/>'
                                 '<w14:ligatures xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" '
                                 f'w14:val="{ligatures}"/>')
                        paragraphs.append(edge.para(edge.run('Word-made Hello World 0123456789', props),
                                                    '<w:rPr>' + props + '</w:rPr>'))
            name = f'yu-mincho-opentype-{"default" if enabled is None else enabled}-compat{compatibility}'
            edge.write(name, ''.join(paragraphs), compatibility)
            target = edge.ROOT / 'public/fixtures' / f'word-windows-edge-{name}.docx'
            with ZipFile(target) as archive:
                parts = [(info, archive.read(info.filename)) for info in archive.infolist()]
            with ZipFile(target, 'w') as archive:
                for info, content in parts:
                    if info.filename == 'word/settings.xml' and enabled is not None:
                        content = content.replace(b'</w:compat>',
                            ('<w:compatSetting w:name="enableOpenTypeFeatures" '
                             'w:uri="http://schemas.microsoft.com/office/word" '
                             f'w:val="{enabled}"/></w:compat>').encode())
                    archive.writestr(info, content)


if __name__ == '__main__':
    main()
