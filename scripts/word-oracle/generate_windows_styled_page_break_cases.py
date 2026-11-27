"""Generate Windows Word probes for styled page breaks and column boundaries."""

import generate_windows_edge_cases as edge
from generate_windows_page_break_cases import (
    BREAK_BEFORE, INDENT, MODES, PAGE, PAGE_GEOMETRY, SOFT, SPACING, para, run, text_run,
)

PREFIX = 'word-windows-edge-styled-page-break-'
COLUMN = '<w:br w:type="column"/>'
NO_BREAK = '<w:pageBreakBefore w:val="0"/>'
COLUMNS = PAGE_GEOMETRY + '<w:cols w:num="2" w:space="720"/>'


def style(identifier, parent, properties=''):
    return (
        f'<w:style w:type="paragraph" w:styleId="{identifier}">'
        f'<w:name w:val="{identifier}"/><w:basedOn w:val="{parent}"/>'
        f'<w:pPr>{properties}</w:pPr></w:style>'
    )


STYLES = (
    style('BreakBase', 'Normal', BREAK_BEFORE + SPACING + INDENT)
    + style('BreakChild', 'BreakBase')
    + style('BreakDisabled', 'BreakChild', NO_BREAK)
    + style('BreakRestored', 'BreakDisabled', BREAK_BEFORE)
)


def styled(identifier, content, properties=''):
    return para(content, f'<w:pStyle w:val="{identifier}"/>' + properties)


def section(geometry):
    return f'<w:sectPr>{geometry}</w:sectPr>'


def section_start(kind, geometry):
    return f'<w:type w:val="{kind}"/>' + geometry


def fixed_height_lines(count):
    return para(
        text_run(*(['LINE', SOFT] * (count - 1) + ['LAST'])),
        '<w:spacing w:line="360" w:lineRule="exact"/>',
    )


def cases():
    head = para(run('HEAD'), SPACING)
    tail = para(run('TAIL'))
    text = run('TARGET')
    inherited = styled('BreakChild', text)
    without_break = styled('BreakDisabled', text)
    next_section = section(section_start('nextPage', PAGE_GEOMETRY))
    final_manual = para(text_run('HEAD', PAGE), SPACING)
    section_final_manual = para(text_run('HEAD', PAGE), SPACING + next_section)
    break_only_with_spacing = para(text_run(PAGE), SPACING)
    at_bottom = fixed_height_lines(36)
    near_bottom = fixed_height_lines(35)
    wider_top_margin = PAGE_GEOMETRY.replace('w:top="1440"', 'w:top="1920"')
    narrower_page = PAGE_GEOMETRY.replace('w:w="12240"', 'w:w="11520"')
    section_break_only = para(text_run(PAGE), section(PAGE_GEOMETRY))
    hidden = '<w:vanish/>'
    return {
        'direct-control': (head + para(text, BREAK_BEFORE + SPACING + INDENT) + tail, PAGE_GEOMETRY),
        'named': (head + styled('BreakBase', text) + tail, PAGE_GEOMETRY),
        'based-on': (head + inherited + tail, PAGE_GEOMETRY),
        'direct-off': (head + styled('BreakChild', text, NO_BREAK) + tail, PAGE_GEOMETRY),
        'style-off': (head + styled('BreakDisabled', text) + tail, PAGE_GEOMETRY),
        'style-reenabled': (head + styled('BreakRestored', text) + tail, PAGE_GEOMETRY),
        'document-start': (inherited + tail, PAGE_GEOMETRY),
        'section-start': (para(run('HEAD'), next_section) + inherited + tail, PAGE_GEOMETRY),
        'empty': (head + styled('BreakChild', '') + tail, PAGE_GEOMETRY),
        'consecutive': (head + inherited + styled('BreakChild', run('NEXT')) + tail, PAGE_GEOMETRY),
        'previous-manual': (final_manual + inherited + tail, PAGE_GEOMETRY),
        'previous-manual-direct': (
            final_manual + para(text, BREAK_BEFORE + SPACING + INDENT) + tail,
            PAGE_GEOMETRY,
        ),
        'previous-manual-split-run': (
            para(run('HEAD') + text_run(PAGE), SPACING) + inherited + tail,
            PAGE_GEOMETRY,
        ),
        'previous-break-only': (
            head + para(text_run(PAGE)) + inherited + tail, PAGE_GEOMETRY,
        ),
        'previous-break-only-direct': (
            head + para(text_run(PAGE)) + para(text, BREAK_BEFORE + SPACING + INDENT) + tail,
            PAGE_GEOMETRY,
        ),
        'previous-manual-style-off': (final_manual + without_break + tail, PAGE_GEOMETRY),
        'previous-soft-then-manual': (
            para(text_run('HEAD', SOFT, PAGE), SPACING) + inherited + tail, PAGE_GEOMETRY,
        ),
        'previous-double-manual': (
            para(text_run('HEAD', PAGE, PAGE), SPACING) + inherited + tail, PAGE_GEOMETRY,
        ),
        'previous-manual-empty-run': (
            para(text_run('HEAD', PAGE) + '<w:r><w:rPr><w:i/></w:rPr></w:r>', SPACING)
            + inherited + tail, PAGE_GEOMETRY,
        ),
        'previous-manual-whitespace': (
            para(text_run('HEAD', '<w:t xml:space="preserve"> </w:t>', PAGE), SPACING)
            + inherited + tail, PAGE_GEOMETRY,
        ),
        'previous-manual-section-start': (
            section_final_manual + inherited + tail, PAGE_GEOMETRY,
        ),
        'previous-manual-section-style-off': (
            section_final_manual + without_break + tail, PAGE_GEOMETRY,
        ),
        'previous-break-only-section': (
            head + para(text_run(PAGE), next_section) + inherited + tail, PAGE_GEOMETRY,
        ),
        'previous-manual-section-content-after': (
            para(text_run('HEAD', PAGE, 'MORE'), SPACING + next_section) + inherited + tail,
            PAGE_GEOMETRY,
        ),
        **{
            f'previous-manual-section-{kind.lower()}': (
                section_final_manual + inherited + tail, section_start(kind, PAGE_GEOMETRY),
            )
            for kind in ('continuous', 'oddPage', 'evenPage')
        },
        'leading-manual': (head + styled('BreakChild', text_run(PAGE, 'TARGET')) + tail, PAGE_GEOMETRY),
        'natural-top': (at_bottom + inherited + tail, PAGE_GEOMETRY),
        'first-column': (head + inherited + tail, COLUMNS),
        'second-column': (para(text_run('HEAD', COLUMN)) + para(run('COLUMN')) + inherited + tail, COLUMNS),
        'second-column-top': (para(text_run('HEAD', COLUMN)) + inherited + tail, COLUMNS),
        'next-column-single': (
            para(text_run('HEAD', PAGE), SPACING + section(PAGE_GEOMETRY)) + without_break + tail,
            section_start('nextColumn', PAGE_GEOMETRY),
        ),
        'next-column-double': (
            para(text_run('HEAD', PAGE), SPACING + section(COLUMNS)) + without_break + tail,
            section_start('nextColumn', COLUMNS),
        ),
        'continuous-margin-change': (
            para(text_run('HEAD', PAGE), SPACING + section(PAGE_GEOMETRY)) + without_break + tail,
            section_start('continuous', wider_top_margin),
        ),
        'continuous-columns-change': (
            para(text_run('HEAD', PAGE), SPACING + section(PAGE_GEOMETRY)) + without_break + tail,
            section_start('continuous', COLUMNS),
        ),
        'next-page-width-change': (
            para(text_run('HEAD', PAGE), SPACING + section(PAGE_GEOMETRY)) + inherited + tail,
            narrower_page,
        ),
        'next-page-break-only-style-off': (
            head + section_break_only + without_break + tail, PAGE_GEOMETRY,
        ),
        'next-page-exact-bottom': (at_bottom + section_break_only + inherited + tail, PAGE_GEOMETRY),
        'next-page-near-bottom': (near_bottom + section_break_only + inherited + tail, PAGE_GEOMETRY),
        'break-only-after-spacing': (head + break_only_with_spacing + inherited + tail, PAGE_GEOMETRY),
        'break-only-section-after-spacing': (
            head + para(text_run(PAGE), SPACING + next_section) + inherited + tail, PAGE_GEOMETRY,
        ),
        **{
            name: (
                para(previous, SPACING + section(PAGE_GEOMETRY)) + para(text) + tail,
                section_start('continuous', PAGE_GEOMETRY),
            )
            for name, previous in (
                ('visible-manual-hidden-after', text_run('HEAD', PAGE) + run('HIDDEN', hidden)),
                ('hidden-manual', run('HEAD') + f'<w:r><w:rPr>{hidden}</w:rPr>{PAGE}</w:r>'),
                ('hidden-whole-line', f'<w:r><w:rPr>{hidden}</w:rPr><w:t>HEAD</w:t>{PAGE}</w:r>'),
            )
        },
        **{
            f'previous-after-{after}-{kind}': (
                para(text_run('HEAD', PAGE), SPACING.replace('320', str(after)) + section_marker)
                + following + tail,
                PAGE_GEOMETRY,
            )
            for after in (0, 160)
            for kind, section_marker, following in (
                ('adjacent', '', para(text, BREAK_BEFORE + SPACING + INDENT)),
                ('section-marked', next_section, inherited),
                ('section-plain', next_section, without_break),
            )
        },
    }


if __name__ == '__main__':
    for mode in MODES:
        for name, (body, geometry) in cases().items():
            edge.write(
                f'styled-page-break-{name}-compat{mode}', body, mode,
                section_properties=geometry, paragraph_styles=STYLES,
            )
