"""Compare DOCX page-break layout and vector PDF with Windows Word."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from zipfile import ZipFile

import pymupdf

from check_platform_coordinates import (
    ROOT, assert_pending_pdf_text, compare, emf_glyphs, extract_fonts, pdf_origins, require,
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
from generate_windows_floating_table_wrap_cases import (
    PREFIX as FLOATING_WRAP_PREFIX, cases as floating_wrap_cases,
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
CAPTURES = (
    ('manual-page-breaks', 'f2c15c0c2414b557b653687f8bb355c59740163e6add384e1825a8a8d6394fdb',
     'word-windows-edge-page-break-', manual_cases, ()),
    ('styled-page-breaks', '686f4826e19a1af9dbd12296388bb672364e2a6da71f4d0f207d180247785883',
     STYLED_PREFIX, styled_cases, ()),
    ('table-page-breaks', '4ef3859c6fffd2a75cc1a2249c892b6e8bdeebe5d70cc6b7f0df31faf2af8d93',
     TABLE_PREFIX, table_cases, ('--painted-table-headers',)),
    ('vertical-table-breaks', '4a4efe07c6bfea1a917d769e8e83b03771e21c22346a401fd479ea617c9afdc3',
     VERTICAL_PREFIX, vertical_cases, ('--unclipped-tables',)),
    ('floating-table-breaks', '1248e0018b2b0be91b1edd5b979b885eb40b0bf29cf9c27119244c2b6974a04c',
     FLOATING_PREFIX, floating_cases, ('--painted-table-headers',)),
    ('floating-table-continuations', '7218048a1b7352bc06dd1d97af0d31ec9b3baf05ae96a5c8ccf499717baf3d26',
     FLOATING_CONTINUATION_PREFIX, floating_continuation_cases, ('--painted-table-headers',)),
    ('floating-table-wrap', 'd0de67e96a9f4085373fddb908b350544f3a98a174ba77ed9a429f278b01e497',
     FLOATING_WRAP_PREFIX, floating_wrap_cases, ('--painted-table-headers',)),
    ('floating-table-overlap', '3948a1997ca5de5477d026869a7ba404afa09e3f8eeebbd8238f0d2965fd3236',
     FLOATING_OVERLAP_PREFIX, floating_overlap_cases, ('--painted-table-headers',)),
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
    check_screen = capture_path.parent.name in (
        'floating-table-breaks', 'floating-table-continuations', 'floating-table-wrap',
        'floating-table-overlap',
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
