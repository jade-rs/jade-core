"""Deterministic Windows decimal-tab probes with original Word captures."""
import generate_windows_edge_cases as edge


CASES = {
    'decimal-tracking': ['12.34', '-12.34', '1,234.56', 'USD 12.34', '12 EUR', '12/34', '12:34', 'A. B', '(12.34)', '1234.56.78'],
    'decimal-suffix': ['12 EUR', '12%', '$12', 'USD 12', '12/34', '12:34', '12-34', '12,345', '12,34', '12 34', '1A2', 'A12B', 'A B12', '12e3', '+12', '-12', '(12)', '12,34X', '1,234.56X'],
    'decimal-separators': ['12 EUR.5', '1A2.3', '12/34.56', '12 34.56', 'USD 12 EUR.5', 'ABC.DEF', 'ABC:DEF', 'ABC12DEF.56', '12,,34X', '12, EUR', '(12.3)', '1 234,56', '.', '--', 'A12,34B.5'],
}


def main():
    for name, texts in CASES.items():
        paragraphs = []
        for spacing in ([None] if name == 'decimal-separators' else [-20, 0, 30]):
            for text in texts:
                props = '' if spacing is None else f'<w:spacing w:val="{spacing}"/>'
                content = (edge.run('Start') + '<w:r><w:tab/></w:r>'
                           + edge.run(text[:2], props) + edge.run(text[2:], props)
                           + '<w:r><w:tab/></w:r>' + edge.run('End'))
                paragraphs.append(edge.para(content, '<w:tabs><w:tab w:val="decimal" w:pos="2880"/><w:tab w:val="left" w:pos="5760"/></w:tabs>'))
        edge.write(name, ''.join(paragraphs))


if __name__ == '__main__':
    main()
