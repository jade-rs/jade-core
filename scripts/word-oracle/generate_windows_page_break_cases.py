"""Generate Windows Word witnesses for manual page breaks and paragraph boundaries."""
import generate_windows_edge_cases as edge
from generate_windows_edge_cases import run

MODES = (12, 14, 15)
PAGE = '<w:br w:type="page"/>'
SAVED = '<w:lastRenderedPageBreak/>'
INDENT = '<w:ind w:left="240" w:firstLine="360"/>'
SPACING = '<w:spacing w:before="240" w:after="320" w:line="360" w:lineRule="exact"/>'
BREAK_BEFORE = '<w:pageBreakBefore/>'
KEEP_LINES = '<w:keepLines/>'
SOFT = '<w:br/>'
HANGING = '<w:ind w:left="600" w:hanging="360"/>'
PAGE_GEOMETRY = ('<w:pgSz w:w="12240" w:h="15840"/>'
                 '<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"'
                 ' w:header="720" w:footer="720" w:gutter="0"/>')
NEXT_SECTION = f'<w:sectPr><w:type w:val="nextPage"/>{PAGE_GEOMETRY}</w:sectPr>'
NUM_PR = '<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>'
NUMBERING = (
    f'<w:numbering xmlns:w="{edge.NS}"><w:abstractNum w:abstractNumId="0">'
    '<w:multiLevelType w:val="singleLevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/>'
    '<w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/>'
    '<w:pPr><w:tabs><w:tab w:val="num" w:pos="600"/></w:tabs>'
    '<w:ind w:left="600" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>'
    '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>'
)


def para(content, properties=''):
    return f'<w:p><w:pPr>{properties}</w:pPr>{content}</w:p>'


def text_run(*children):
    return '<w:r>' + ''.join(
        child if child.startswith('<') else f'<w:t>{child}</w:t>'
        for child in children
    ) + '</w:r>'


def cases():
    prefix = para(run('HEAD'))
    tail = para(run('TAIL'))
    same_run = para(text_run('BEFORE', PAGE, 'AFTER'), SPACING + INDENT)
    split_runs = para(run('BEFORE') + text_run(PAGE) + run('AFTER'), SPACING + INDENT)
    lead = para(run('BEFORE'), SPACING + INDENT)
    almost_full = para(
        text_run(*(['LINE', SOFT] * 34 + ['LINE'])),
        '<w:spacing w:line="360" w:lineRule="exact"/>',
    )
    return {
        'same-run': prefix + same_run + tail,
        'split-runs': prefix + split_runs + tail,
        'trailing': prefix + para(text_run('BEFORE', PAGE), SPACING + INDENT) + tail,
        'trailing-spaced-next': prefix + para(run('BEFORE') + text_run(PAGE), SPACING + INDENT)
        + para(run('AFTER'), SPACING + INDENT) + tail,
        'leading': prefix + lead + para(text_run(PAGE, 'AFTER'), SPACING + INDENT) + tail,
        'standalone': prefix + lead + para(text_run(PAGE), SPACING + INDENT)
        + para(run('AFTER'), SPACING + INDENT) + tail,
        'paragraph-property': prefix + lead + para(run('AFTER'), BREAK_BEFORE + SPACING + INDENT) + tail,
        'paragraph-property-after-zero': prefix + para(run('BEFORE'), SPACING.replace('320', '0') + INDENT)
        + para(run('AFTER'), BREAK_BEFORE + SPACING + INDENT) + tail,
        'paragraph-property-after-eight': prefix + para(run('BEFORE'), SPACING.replace('320', '160') + INDENT)
        + para(run('AFTER'), BREAK_BEFORE + SPACING + INDENT) + tail,
        'document-start': para(text_run(PAGE, 'AFTER'), SPACING + INDENT) + tail,
        'document-end': prefix + para(text_run('BEFORE', PAGE), SPACING + INDENT),
        'consecutive': prefix + para(text_run('BEFORE', PAGE, PAGE, 'AFTER'), SPACING + INDENT) + tail,
        'trailing-consecutive': prefix + para(run('BEFORE') + text_run(PAGE) + text_run(PAGE), SPACING)
        + tail,
        'trailing-empty-next': prefix + para(text_run('BEFORE', PAGE), SPACING + INDENT)
        + para('') + tail,
        'saved-marker': prefix + para(text_run('BEFORE', SAVED, 'AFTER'), SPACING + INDENT) + tail,
        'manual-and-saved': prefix + para(text_run('BEFORE', PAGE, SAVED, 'AFTER'), SPACING + INDENT)
        + tail,
        'keep-lines': prefix + para(text_run('BEFORE', PAGE, 'AFTER'), KEEP_LINES + SPACING + INDENT)
        + tail,
        'leading-split-runs': prefix + lead + para(text_run(PAGE) + run('AFTER'), SPACING + INDENT)
        + tail,
        'leading-saved-marker': prefix + lead
        + para(text_run(SAVED) + text_run(PAGE, 'AFTER'), SPACING + INDENT) + tail,
        'leading-consecutive': prefix + lead
        + para(text_run(PAGE, PAGE, 'AFTER'), SPACING + INDENT) + tail,
        'document-start-consecutive': para(text_run(PAGE, PAGE, 'AFTER'), SPACING + INDENT) + tail,
        'leading-page-break-before': prefix + lead
        + para(text_run(PAGE, 'AFTER'), BREAK_BEFORE + SPACING + INDENT) + tail,
        'leading-after-terminal': prefix + para(text_run('BEFORE', PAGE), SPACING + INDENT)
        + para(text_run(PAGE, 'AFTER'), SPACING + INDENT) + tail,
        'leading-hanging': prefix + lead + para(text_run(PAGE, 'AFTER', SOFT, 'NEXT'), SPACING + HANGING)
        + tail,
        'soft-then-page': prefix + lead + para(text_run(SOFT, PAGE, 'AFTER'), SPACING + INDENT) + tail,
        'space-then-page': prefix + lead
        + para(text_run('<w:t xml:space="preserve"> </w:t>', PAGE, 'AFTER'), SPACING + INDENT)
        + tail,
        'leading-wrap': prefix + lead + para(
            text_run(PAGE) + run('ONE TWO THREE FOUR FIVE SIX SEVEN EIGHT NINE TEN'),
            SPACING + '<w:ind w:right="7440" w:firstLine="720"/>',
        ) + tail,
        'leading-near-bottom': almost_full
        + para(text_run(PAGE, 'AFTER'), SPACING + INDENT) + tail,
        'leading-section-start': para(run('HEAD'), NEXT_SECTION)
        + para(text_run(PAGE, 'AFTER'), SPACING + INDENT) + tail,
        'leading-document-break-before': para(
            text_run(PAGE, 'AFTER'), BREAK_BEFORE + SPACING + INDENT,
        ) + tail,
        'leading-keep-next': prefix + lead
        + para(text_run(PAGE, 'AFTER'), '<w:keepNext/>' + SPACING + INDENT) + tail,
        'split-styled-runs': prefix + para(
            run('BEFORE', '<w:b/>') + text_run(PAGE) + run('AFTER', '<w:i/>'),
            SPACING + INDENT,
        ) + tail,
        'numbered-document-start': para(text_run(PAGE, 'AFTER'), NUM_PR) + tail,
        'numbered-leading': para(run('BEFORE'), NUM_PR) + para(text_run(PAGE, 'AFTER'), NUM_PR)
        + tail,
    }


if __name__ == '__main__':
    for mode in MODES:
        for name, body in cases().items():
            edge.write(
                f'page-break-{name}-compat{mode}', body, mode, section_properties=PAGE_GEOMETRY,
                numbering=NUMBERING if name.startswith('numbered-') else None,
            )
