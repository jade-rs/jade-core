"""Compare DOCX page-break layout and vector PDF with Windows Word."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from zipfile import ZipFile

import pymupdf

from check_platform_coordinates import (
    ROOT, assert_original_vector_borders, assert_pending_pdf_text, compare, emf_glyphs,
    extract_fonts, pdf_origins, require,
)
from check_windows_font_coordinates import require_original_font_programs_registered
from generate_windows_floating_table_break_cases import (
    PREFIX as FLOATING_PREFIX, cases as floating_cases,
)
from generate_windows_floating_table_continuation_cases import (
    PREFIX as FLOATING_CONTINUATION_PREFIX, cases as floating_continuation_cases,
)
from generate_windows_floating_table_overlap_cases import (
    PREFIX as FLOATING_OVERLAP_PREFIX, cases as floating_overlap_cases,
)
from generate_windows_floating_table_word_wrap_cases import (
    PREFIX as FLOATING_WORD_WRAP_PREFIX, cases as floating_word_wrap_cases,
)
from generate_windows_floating_table_wrap_cases import (
    PREFIX as FLOATING_WRAP_PREFIX, cases as floating_wrap_cases,
)
from generate_windows_nested_table_alignment_cases import (
    PREFIX as NESTED_ALIGNMENT_PREFIX, cases as nested_alignment_cases,
)
from generate_windows_nested_table_depth_cases import (
    PREFIX as NESTED_DEPTH_PREFIX, cases as nested_depth_cases,
)
from generate_windows_nested_table_overflow_cases import (
    PREFIX as NESTED_OVERFLOW_PREFIX, context_cases as nested_overflow_cases,
    row_height_cases as nested_row_height_cases,
)
from generate_windows_nested_table_page_break_cases import (
    PREFIX as NESTED_TABLE_PREFIX, cases as nested_table_cases,
)
from generate_windows_nested_table_preferred_cases import (
    PREFIX as NESTED_PREFERRED_PREFIX, cases as nested_preferred_cases,
)
from generate_windows_page_break_cases import MODES, cases as manual_cases
from generate_windows_styled_page_break_cases import PREFIX as STYLED_PREFIX, cases as styled_cases
from generate_windows_table_page_break_cases import PREFIX as TABLE_PREFIX, cases as table_cases
from generate_windows_vertical_table_break_cases import (
    PREFIX as VERTICAL_PREFIX, cases as vertical_cases,
)

WORD_VERSION = '16.0.19127.20936'
FIXTURES = ROOT / 'public/fixtures'
CAPTURE_ROOT = ROOT / 'tests/word-oracle/windows'
FLOATING_RULE_CASES = {
    f'{FLOATING_OVERLAP_PREFIX}right-{rule}-compat{mode}'
    for rule in ('left-rule', 'right-rule', 'border', 'asymmetric-rules')
    for mode in MODES
}


def nested_resized_cases():
    return {f'{layout}-{width}' for layout in ('autofit', 'fixed')
            for width in ('absolute', 'auto', 'full')}


def nested_percent_resize_cases():
    return {'w2059', 'w2060', 'w2061', 'w2160'}


def spaced_border_content_cases():
    rules = ('no-rule', 'only-left', 'only-right', 'only-inner', 'thin-all',
             'strong-all', 'cell-sides', 'frame-and-cell-sides')
    return {f'{phase}-{rule}' for phase in ('before', 'resized') for rule in rules}


def nested_table_advance_cases():
    samples = ('arial-a', 'arial-av', 'arial-f', 'arial-w', 'calibri-w', 'times-w')
    return {f'{sample}-{weight}' for sample in samples for weight in ('regular', 'italic')}


def nested_autofit_advance_cases():
    return {f'{phase}-{case}' for phase in ('before', 'after')
            for case in nested_table_advance_cases()}


def nested_autofit_resize_cases():
    bases = {f'n{length}-{sample}' for length in (22, 23, 24)
             for sample in ('edge', 'rightedge')}
    return ({f'before-{base}' for base in bases}
            | {f'after-{base}-w{width}' for base in bases for width in (55, 65, 75, 90)})


def nested_autofit_floor_cases():
    widths = (8000, 8200, 8280, 8340, 8380, 8400, 8410, 8420, 8440, 8460, 8480, 8500)
    return {f'{sample}-{width}' for sample in ('edge', 'rightedge') for width in widths}


def nested_autofit_twip_cases():
    widths = {'absolute': range(1496, 1500), 'percentage': range(4644, 4647)}
    return {f'{kind}-{sample}-{width}' for kind, values in widths.items()
            for sample in ('edge', 'rightedge') for width in values}


def nested_autofit_parent_save_cases():
    return {'same-run', 'separate-run', 'separate-paragraph', 'grid-change'}


def nested_autofit_batch_resize_cases():
    return {f'{kind}-n{length}-{position}' for kind in ('repeat', 'shorten')
            for length in (22, 23, 24) for position in ('edge', 'rightedge')}


def nested_fixed_parent_width_cases():
    widths = (2500, 2823, 2954, 2963, 3023, 3043)
    return {f'{sample}-{width}' for sample in ('edge', 'rightedge') for width in widths}


def collapsed_border_content_cases():
    regular = ('no-rules', 'weak', 'strong', 'one-inside', 'one-outer',
               'explicit-zero', 'explicit-small', 'explicit-large')
    asymmetric = ('balanced-threshold', 'left-six', 'left-three',
                  'only-inside-right-padding', 'right-six')
    return ({f'plain-{case}' for case in regular}
            | {f'{kind}-{case}' for kind in ('nested', 'resized') for case in regular + asymmetric})


CAPTURES = (
    ('manual-page-breaks', 'f2c15c0c2414b557b653687f8bb355c59740163e6add384e1825a8a8d6394fdb',
     'word-windows-edge-page-break-', manual_cases, ()),
    ('styled-page-breaks', '686f4826e19a1af9dbd12296388bb672364e2a6da71f4d0f207d180247785883',
     STYLED_PREFIX, styled_cases, ()),
    ('table-page-breaks', '4ef3859c6fffd2a75cc1a2249c892b6e8bdeebe5d70cc6b7f0df31faf2af8d93',
     TABLE_PREFIX, table_cases, ('--painted-table-headers',)),
    ('nested-table-page-breaks', '67b36135e2762d18e21827129c283296342665742b58dbfc661846949a8ffde2',
     NESTED_TABLE_PREFIX, nested_table_cases, ('--painted-table-headers',)),
    ('nested-table-overflow', 'be6a8f683f4eea6c88d8e6afc3eaaa7a93349937bf0a79bc33f62163e8dad30e',
     NESTED_OVERFLOW_PREFIX, nested_overflow_cases, ('--painted-table-headers',)),
    ('nested-table-row-heights', '7636bbce032a413ba14cfe9ca0bfb2c69830c497b6169e68a1f3356867dc8d64',
     NESTED_OVERFLOW_PREFIX, nested_row_height_cases, ('--unclipped-tables',)),
    ('nested-table-alignment', '3c082a14852f0c2c1725ba83cc676eb2c5fcb4a2cd653b7d1a5b3716520f1e02',
     NESTED_ALIGNMENT_PREFIX, nested_alignment_cases, ('--unclipped-tables',)),
    ('nested-table-depth', '9e02614367b540168a00c2f8f11a4d83396930696c5876a0a6f5f873ede50d42',
     NESTED_DEPTH_PREFIX, nested_depth_cases, ('--unclipped-tables',)),
    ('nested-table-preferred-widths', '1ee621a881cba218780c9423e1566b9db830d72e0cca1e8675e630e870e2681d',
     NESTED_PREFERRED_PREFIX, nested_preferred_cases, ('--unclipped-tables',)),
    ('nested-table-resize', '40be43660a428dc8783e2b16dbbc26f6883c73342cdda3762c00fa7ee5d62bfa',
     'word-windows-edge-nested-resized-', nested_resized_cases, ('--unclipped-tables',)),
    ('table-cell-border-content', '3e90959682d1ef097e157ab2cbe17af8a996b66080be561e9fdd39b79c5108b4',
     'word-windows-edge-border-content-', collapsed_border_content_cases, ('--unclipped-tables',)),
    ('nested-table-percent-resize', '23ec5828f242906c655fb6e636915ce5bebdc0ff6fd49746110e1076fba84a88',
     'word-windows-edge-nested-percent-resized-', nested_percent_resize_cases, ('--unclipped-tables',)),
    ('spaced-table-border-content', '2ae9c231f0f24f1db3ffb1dcf4d2750e0cd26d1532db6875e3478bba55f5732b',
     'word-windows-edge-spaced-border-content-', spaced_border_content_cases, ('--unclipped-tables',)),
    ('nested-autofit-advances', 'f9c1d198abd4127a2b18565b9a5ed3c0d6d6bbec3a8127750afb3dbd08255d30',
     'word-windows-edge-nested-autofit-advance-', nested_autofit_advance_cases, ('--unclipped-tables',)),
    ('nested-autofit-resize', 'ff433fbc065d00c29fedcf9b4883aed2315360573e697f3a9628b0ad41b4cdce',
     'word-windows-edge-nested-autofit-resize-', nested_autofit_resize_cases, ('--unclipped-tables',)),
    ('nested-autofit-floors', '1cee360c6106b320e55b1bee7305b65690d451635396683b3d28b346ba4ce173',
     'word-windows-edge-nested-autofit-floor-', nested_autofit_floor_cases, ('--unclipped-tables',)),
    ('nested-autofit-twip-rounding', '9d245fc3f85d693bf8ebe5ba8d0e68f0f135e0ad4f2a874f3129e228f6bac3cc',
     'word-windows-edge-nested-autofit-twip-', nested_autofit_twip_cases, ('--unclipped-tables',)),
    ('nested-autofit-parent-save', '20baa90e1c64349caa5e4f62d176c79ae90d7613b52d7ada1714fa53a7992587',
     'word-windows-edge-nested-autofit-parent-save-', nested_autofit_parent_save_cases, ('--unclipped-tables',)),
    ('nested-autofit-batch-resize', 'e5b5025d5a739bb0d0600b7f8480e4445dfd6e9bbebe5d326094ab22e986165b',
     'word-windows-edge-nested-autofit-batch-', nested_autofit_batch_resize_cases, ('--unclipped-tables',)),
    ('nested-fixed-advances', '842bd436081f746899214dc718aa4371d16ae5d0ab7049a61c5742960f3e0920',
     'word-windows-edge-nested-fixed-advance-', nested_table_advance_cases, ('--unclipped-tables',)),
    ('nested-fixed-parent-widths', '88c2b573ec528d89c9797815aef9c6b1373b1b4f53985fc2420cc02f4d379595',
     'word-windows-edge-nested-fixed-floor-', nested_fixed_parent_width_cases, ('--unclipped-tables',)),
    ('vertical-table-breaks', '4a4efe07c6bfea1a917d769e8e83b03771e21c22346a401fd479ea617c9afdc3',
     VERTICAL_PREFIX, vertical_cases, ('--unclipped-tables',)),
    ('floating-table-breaks', '1248e0018b2b0be91b1edd5b979b885eb40b0bf29cf9c27119244c2b6974a04c',
     FLOATING_PREFIX, floating_cases, ('--painted-table-headers',)),
    ('floating-table-continuations', '7218048a1b7352bc06dd1d97af0d31ec9b3baf05ae96a5c8ccf499717baf3d26',
     FLOATING_CONTINUATION_PREFIX, floating_continuation_cases, ('--painted-table-headers',)),
    ('floating-table-wrap', 'd0de67e96a9f4085373fddb908b350544f3a98a174ba77ed9a429f278b01e497',
     FLOATING_WRAP_PREFIX, floating_wrap_cases, ('--painted-table-headers',)),
    ('floating-table-overlap', '32d44eaed9065a08a61c4fd5a4f19f8eafb6b8bd5288316da32292228c306cc2',
     FLOATING_OVERLAP_PREFIX, floating_overlap_cases, ('--painted-table-headers',)),
    ('floating-table-word-wrap', 'b69afb991ee5fa5d2ca8983aee7597978a1e1e77f5ca3c3266ec383dcb7a5e1a',
     FLOATING_WORD_WRAP_PREFIX, floating_word_wrap_cases, ('--painted-table-headers',)),
)


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def pdf_page_sizes(path):
    with pymupdf.open(path) as pdf:
        return [(page.rect.width, page.rect.height) for page in pdf]


def check_capture(capture_path, capture_hash, remaining, output_dir,
                  native_executable, system_fonts, native_options):
    require(sha256(capture_path.read_bytes()) == capture_hash, f'{capture_path}: Word capture changed')
    failures = []
    fixtures = pages = glyphs = 0
    check_spaced_borders = capture_path.parent.name == 'spaced-table-border-content'
    check_screen = capture_path.parent.name in (
        'nested-table-page-breaks', 'nested-table-overflow', 'nested-table-row-heights',
        'nested-table-alignment', 'nested-table-depth', 'nested-table-preferred-widths',
        'nested-table-resize', 'table-cell-border-content', 'nested-table-percent-resize',
        'spaced-table-border-content',
        'nested-autofit-advances', 'nested-autofit-resize', 'nested-autofit-floors',
        'nested-autofit-twip-rounding', 'nested-autofit-parent-save', 'nested-autofit-batch-resize',
        'nested-fixed-advances', 'nested-fixed-parent-widths',
        'floating-table-breaks', 'floating-table-continuations', 'floating-table-wrap',
        'floating-table-overlap', 'floating-table-word-wrap',
    )
    with ZipFile(capture_path) as bundle:
        capture = json.loads(bundle.read('word/capture.json').decode('utf-8-sig'))
        require(capture['product_version'] == WORD_VERSION, 'Incorrect Word version')
        for original in capture['documents']:
            name = original['name']
            require(name in remaining, f'{name}: unknown or duplicate fixture')
            remaining.remove(name)
            source = FIXTURES / f'{name}.docx'
            require(sha256(source.read_bytes()) == original['input_sha256'],
                    f'{name}: Word measured different DOCX bytes')
            if 'before_fixture' in original:
                before = FIXTURES / f'{original["before_fixture"]}.docx'
                require(sha256(before.read_bytes()) == original['before_input_sha256'],
                        f'{name}: Word edited different DOCX bytes')
            pdf_bytes = bundle.read(f'word/{name}.pdf')
            require(sha256(pdf_bytes) == original['pdf_sha256'], f'{name}: Word PDF changed')
            case_dir = output_dir / name
            case_dir.mkdir(parents=True, exist_ok=True)
            word_pdf = case_dir / 'word.pdf'
            word_pdf.write_bytes(pdf_bytes)
            word = pdf_origins(word_pdf)
            require(word['page_count'] == original['page_count'],
                    f'{name}: Word PDF and Word page counts differ')
            if check_screen:
                metafiles = original['metafiles']
                require([page['page'] for page in metafiles] == list(range(1, word['page_count'] + 1)),
                        f'{name}: incomplete Word print layout capture')
                screen = {'page_count': word['page_count'], 'glyphs': []}
                for page in metafiles:
                    data = bundle.read(f'word/{page["file"]}')
                    require(sha256(data) == page['sha256'], f'{name}: Word drawing changed')
                    screen['glyphs'].extend(emf_glyphs(
                        data, page['page'], original['width_pt'], original['height_pt'],
                    ))
            fixtures += 1
            pages += word['page_count']
            glyphs += len(word['glyphs'])
            if native_executable is None:
                continue

            native_json = case_dir / 'native.json'
            native_pdf = case_dir / 'native.pdf'
            argv = [str(native_executable), str(source), str(native_json), 'windows',
                    str(native_pdf), '--without-formatting-marks', *native_options]
            if not system_fonts:
                fonts_dir = case_dir / 'word-fonts'
                if not fonts_dir.exists():
                    extract_fonts(word_pdf, fonts_dir)
                argv.extend(['--fonts-directory', str(fonts_dir)])
            try:
                subprocess.run(argv, check=True, timeout=60)
                snapshot = json.loads(native_json.read_bytes())
                require(snapshot['input_sha256'] == original['input_sha256'],
                        f'{name}: renderer opened different DOCX bytes')
                if not system_fonts:
                    require_original_font_programs_registered(fonts_dir, snapshot)
                require(native_pdf.read_bytes() != pdf_bytes, f'{name}: Word self-comparison')
                painted = pdf_origins(native_pdf)
                require(pdf_page_sizes(word_pdf) == pdf_page_sizes(native_pdf),
                        f'{name}: vector PDF page sizes differ from Word')
                # The snapshot stores list labels in a separate collection.
                # Match its complete glyph set by page and origin. The vector
                # PDF also carries the visible reading order.
                if '--unclipped-tables' in native_options:
                    full_word = pdf_origins(word_pdf, unclipped=True)
                    compare(full_word, snapshot, f'{name} unclipped layout origins')
                    full_painted = pdf_origins(native_pdf, unclipped=True)
                    # Removing clip paths can change extraction order. Compare
                    # those extra glyphs by page and origin, and assert the
                    # printed reading order using the visible projection.
                    compare(full_word, full_painted, f'{name} unclipped vector PDF')
                else:
                    compare(word, snapshot, f'{name} layout origins')
                if check_screen:
                    # Body text and floating content are painted separately.
                    # Preserve every glyph's position and page on both surfaces.
                    compare(screen, snapshot, f'{name} Word print layout')
                else:
                    assert_pending_pdf_text(word, painted, f'{name} vector PDF')
                compare(word, painted, f'{name} vector PDF')
                if name in FLOATING_RULE_CASES or check_spaced_borders:
                    assert_original_vector_borders(
                        word_pdf, native_pdf, name,
                        expected_empty=check_spaced_borders and '-no-rule-' in name,
                    )
            except (ValueError, subprocess.SubprocessError) as error:
                failures.append(str(error))
    require(not remaining, f'Missing Word captures: {sorted(remaining)}')
    require(not failures, '\n'.join(failures))
    print(f'{capture_path.parent.name}: {fixtures} fixtures, {pages} pages, '
          f'{glyphs} glyphs from Word {WORD_VERSION}')


def check(output_dir, native_executable, system_fonts):
    for cohort, digest, prefix, cases, native_options in CAPTURES:
        expected = {
            f'{prefix}{name}-compat{mode}'
            for name in cases()
            for mode in MODES
        }
        check_capture(
            CAPTURE_ROOT / cohort / f'word-{WORD_VERSION}.zip', digest, expected,
            output_dir, native_executable, system_fonts, native_options,
        )


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, required=True)
    parser.add_argument('--system-fonts', action='store_true')
    args = parser.parse_args()
    check(args.output_directory.resolve(),
          args.native_executable.resolve() if args.native_executable else None, args.system_fonts)
