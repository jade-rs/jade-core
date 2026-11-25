"""Assert shared DOCX identity and independent Mac/Windows Word expectations.

Mac rendering is checked by check_mac_word_coordinates.mjs on macOS. This
registry also runs the Windows native renderer against every Windows capture,
including raw EMF, PDF text and independently painted native PDF origins.
"""
import argparse
from collections import Counter, defaultdict, deque
import gzip
import hashlib
import json
import math
import re
from pathlib import Path
import subprocess
import sys
from zipfile import ZipFile

from extract_windows_emf import extract, emf_glyphs

ROOT = Path(__file__).resolve().parents[2]
INDEX = Path('tests/word-oracle/platform-coordinates.json')
PREFERRED_GRID_BORDER_CASES = {
    'probe-width-single-pct-2500-pct-2500',
    'probe-width-unequal-dxa-2400-dxa-2000',
    'probe-width-unequal-pct-2500-auto-0',
    'probe-width-unequal-pct-2500-dxa-2000',
}
# Original physical font face/size and per-glyph XY are fully supported here.
# Keep this separate from logical EMF coordinates: Windows print realization
# can differ from screen metrics. Every case retains its own Word PDF.
PHYSICAL_FONT_CASES = {
    'probe-mixed-sangam-matrix-240',
    'probe-mixed-sangam-matrix-276',

    'probe-devanagari-line-box-devanagari',
    'probe-complex-font-sweep-thonburi',
    'probe-complex-font-metrics-thonburi',
    '2col-mac',
    'align-center-mac',
    'alignment-mac',
    'aptos-ascii-advances-mac',
    'aptos-kerning-mac',
    'aptos-numeric-ranges-mac',
    'aptos-range-compat-14-en-US-mac',
    'aptos-range-compat-15-en-US-mac',
    'ascii-punctuation-mac',
    'borderless-table-mac',
    'cell-alignment-mac',
    'char-spacing-mac',
    'column-break-mac',
    'column-table-fixed-preferred-width-mac',
    'column-table-fixed-preferred-width-smaller-mac',
    'courier-new-line-metrics',
    'courier-new-regular-adjacent',
    'decimal-tabs-mac',
    'direction-with-selection-mac',
    'endnote-mac',
    'eqruby-grid-mac',
    'eqruby-nogrid-mac',
    'exact-and-minimum-lines-mac',
    'floating-inside-outside-mac',
    'floating-simple-position-mac',
    'floating-square-wrap-mac',
    'floating-through-concave-mac',
    'floating-tight-polygon-mac',
    'floating-top-bottom-offset-mac',
    'floating-wrap-sides-mac',
    'floating-z-order-mac',
    'footnote-long-continuation-mac',
    'footnote-mac',
    'footnote-table-each-page-mac',
    'footnote-table-textbox-order-mac',
    'fractional-picture-anchors-mac',
    'header-footer-edge-cases-mac',
    'header-footer-sectionpages-mac',
    'header-footer-sections-mac',
    'header-tab-overflow-long-mac',
    'indents-mac',
    'inline-image-position-grid-mac',
    'inline-image-position-nogrid-mac',
    'inline-image-rotation-table-picture-only-mac',
    'jade-script-baseline-20261030',
    'kana-mac',
    'keep-mac',
    'latin-break-budget-620',
    'latin-break-budget-638',
    'latin-break-budget-645',
    'latin-break-budget-652',
    'latin-break-budget-659',
    'line-numbering-mac',
    'line-spacing-mac',
    'merged-table-over-page-mac',
    'mirror-margins-mac',
    'mirror-margins-parity-mac',
    'nested-after-spacing-oversized-48-0',
    'nested-after-spacing-repeat-header',
    'nested-after-spacing-short-5pt',
    'nested-merged-tables-mac',
    'note-styles-creation-mac',
    'numbering-by-style-mac',
    'numbering-restart-mac',
    'odd-even-stories-mac',
    'paragraph-spacing-mac',
    'probe-column-fixed-dxa-4400',
    'probe-contextual-paragraph-spacing-mode14',
    'probe-contextual-paragraph-spacing-mode15',
    'probe-hyphenation-split-word-sweep-off',
    'probe-paragraph-border-wrap',
    'probe-unequal-left-narrow-first-1-1',
    'probe-unequal-left-narrow-first-2-1',
    'probe-unequal-left-wide-first-1-1',
    'probe-unequal-left-wide-first-2-1',
    'ruby-grid-mac',
    'ruby-nogrid-mac',
    'ruby-wide-grid-mac',
    'ruby-wide-nogrid-mac',
    'run-styles-mac',
    'script-position-mac',
    'section-break-types-mac',
    'sections-mac',
    'story-inline-picture-metrics-mac',
    'table-boundary-cant-split-mac',
    'table-oversized-row-after-spacing-fresh-fit',
    'table-row-after-spacing-short-5pt',
    'table-vertical-merge-page-break-mac',
    'table3x2merged-mac',
    'tabs-geometry-mac',
    'tabs-mac',
    'textbox-body-properties-mac',
    'textbox-exact-line-matrix-mac',
    'textbox-fixed-pitch-matrix-mac',
    'textbox-tall-header-overlap-mac',
    'toc-instrtext-edge-cases-mac',
    'toc-sequence-page-number-mac',
    'underline-kinds-mac',
    'unequal-columns-mac',
    'vert-align-sweep-2-browser-mac',
    'vert-align-sweep-browser-mac',
    'widow-control-mac',
    'word-compat-comprehensive-mac',
    'word-mac-probe-distribute-single',
    'word-mac-probe-float-lookahead',
    'word-mac-probe-floating-horizontal-14',
    'word-mac-probe-floating-horizontal-15',
    'word-mac-probe-grid-line-unit',
    'word-mac-probe-grid-line-unit-table',
    'word-mac-probe-indent',
    'word-mac-probe-numbering-label',
    'word-mac-probe-numbering-label-faces',
    'word-mac-probe-object-space-runs-unbalanced',
    'word-mac-probe-pairs-kerned',
    'word-mac-probe-squeeze-conditions-nocompress',
    'word-mac-probe-squeeze-plain',
    'word-mac-probe-vert-align-sweep-2',
    'wrong-num-mac',
    'probe-column-fixed-dxa-6800',
    'probe-width-single-dxa-2400-dxa-2000',
    'probe-continuous-column-explicit-break',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-1',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-2',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-3',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-1',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-2',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-3',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-1',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-2',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-3',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-1',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-2',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-3',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-1',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-2',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-3',
    'probe-fixed-auto-table-grid-2000-cell-pct-1250',
    'probe-fixed-auto-table-grid-1000-cell-pct-2500',
    'probe-fixed-auto-table-grid-2000-cell-pct-5000',
    'probe-fixed-auto-table-grid-4000-cell-pct-2500',
    'probe-unequal-variant-narrow-first-2-9-dxa',
    'probe-width-equal-auto-0-pct-2500',
    'probe-width-single-auto-0-pct-2500',
    'probe-unequal-autofit-wrapped-header',
    'probe-unequal-center-narrow-first-2-9',
    'probe-unequal-center-three-10',
    'probe-table-flow-unequal-start-column-1',
    'probe-table-flow-unequal-start-column-2',
    'probe-unequal-autofit-section-width-matrix',
    'probe-unequal-center-narrow-first-1-1',
    'probe-unequal-center-narrow-first-1-9',
    'probe-unequal-center-three-13',
    'probe-unequal-center-three-17',
    'probe-unequal-center-three-24',
    'probe-unequal-center-three-4',
    'probe-unequal-center-wide-first-1-1',
    'probe-unequal-center-wide-first-1-9',
    'probe-unequal-center-wide-first-2-9',
    'probe-unequal-control-1-16-with-tail',
    'probe-unequal-control-1-16-without-tail',
    'probe-unequal-control-1-21-with-tail',
    'probe-unequal-control-1-21-without-tail',
    'probe-unequal-control-1-22-with-tail',
    'probe-unequal-control-1-22-without-tail',
    'probe-unequal-control-1-23-with-tail',
    'probe-unequal-control-1-23-without-tail',
    'probe-unequal-control-1-24-without-tail',
    'probe-unequal-control-2-12-with-tail',
    'probe-unequal-control-2-12-without-tail',
    'probe-unequal-control-2-19-with-tail',
    'probe-unequal-control-2-19-without-tail',
    'probe-unequal-control-3-14-with-tail',
    'probe-unequal-control-3-14-without-tail',
    'probe-unequal-control-3-8-with-tail',
    'probe-unequal-control-3-8-without-tail',
    'probe-unequal-explicit-paragraph-width-3300',
    'probe-unequal-explicit-paragraph-width-3566',
    'probe-unequal-explicit-paragraph-width-3800',
    'probe-unequal-variant-narrow-first-2-9-center',
    'probe-unequal-variant-narrow-first-2-9-long',
    'probe-unequal-variant-three-24-center',
    'probe-unequal-variant-three-24-long',
    'probe-width-short-narrow-first-1-1',
    'probe-width-short-narrow-first-1-4',
    'probe-width-short-narrow-first-1-5',
    'probe-width-short-narrow-first-1-9',
    'probe-width-short-narrow-first-2-1',
    'probe-width-short-narrow-first-2-4',
    'probe-width-short-narrow-first-2-5',
    'probe-width-short-narrow-first-2-9',
    'probe-width-short-wide-first-1-1',
    'probe-width-short-wide-first-1-4',
    'probe-width-short-wide-first-1-5',
    'probe-width-short-wide-first-1-9',
    'probe-width-short-wide-first-2-1',
    'probe-width-short-wide-first-2-4',
    'probe-width-short-wide-first-2-5',
    'probe-width-short-wide-first-2-9',
    'probe-width-unequal-auto-0-pct-2500',
    'probe-width-unequal-pct-2500-pct-2500',
    'probe-width-unequal-pct-2500-pct-5000',
    'probe-width-unequal-pct-5000-pct-2500',
    'probe-unequal-variant-three-24-left',
    'probe-unequal-variant-three-24-right',
    'probe-second-column-centered-table',
    'probe-table-flow-cell-spacing-start-column-1',
    'probe-table-flow-continuous-start-column-1',
    'probe-table-flow-continuous-start-column-2',
    'probe-width-equal-pct-2500-auto-0',
    'probe-width-equal-pct-2500-dxa-2000',
    'probe-width-equal-pct-2500-pct-2500',
    'probe-width-equal-pct-2500-pct-5000',
    'probe-width-equal-pct-5000-pct-2500',
    'probe-table-flow-continuous-vertical-merge-start-column-1',
    'probe-table-flow-continuous-vertical-merge-start-column-2',
    'probe-table-flow-split-rows-continuous-start-column-1',
    'probe-table-flow-split-rows-start-column-1',
    'probe-table-flow-split-rows-start-column-2',
    'probe-table-flow-start-column-1',
    'probe-table-flow-start-column-2',
    'table-flow-continuous-start-column-1-mac',
    'table-flow-continuous-vertical-merge-start-column-2-mac',
    'table-flow-split-rows-start-column-1-mac',
    'probe-unequal-variant-three-24-dxa',
    'table-bidi-alignment-mac',
    'table-bidi-visual-mac',
    'table-border-conflicts-mac',
    'table-diagonal-borders-mac',
    'table-hidden-rows-mac',
    'table-hide-mark-mac',
    'table-repeating-header-mac',
    'probe-table-flow-unequal-continuous-right-three-columns-start-column-1',
    'probe-table-flow-unequal-continuous-right-three-columns-start-column-2',
    'probe-table-flow-unequal-continuous-right-three-columns-start-column-3',
    'probe-table-flow-unequal-right-start-column-1',
    'probe-table-flow-unequal-right-start-column-2',
    'probe-unequal-left-narrow-first-1-9',
    'probe-unequal-left-narrow-first-2-9',
    'probe-unequal-left-start-column-1',
    'probe-unequal-left-wide-first-1-9',
    'probe-unequal-left-wide-first-2-9',
    'probe-unequal-right-narrow-first-1-1',
    'probe-unequal-right-narrow-first-1-9',
    'probe-unequal-right-narrow-first-2-9',
    'probe-unequal-right-three-10',
    'probe-unequal-right-three-17',
    'probe-unequal-right-three-24',
    'probe-unequal-right-three-4',
    'probe-unequal-right-three-7',
    'probe-unequal-right-wide-first-1-1',
    'probe-unequal-right-wide-first-1-9',
    'probe-unequal-right-wide-first-2-9',
    'probe-unequal-variant-narrow-first-2-9-left',
    'probe-unequal-variant-narrow-first-2-9-right',
    'table-legacy-hmerge-mac',
    'table-cell-fit-text-mac',
    'probe-math-display-alignment-1',
    'probe-math-display-rtl-wrap-1',
    'probe-math-italic-advances-lowercase',
    'probe-math-italic-advances-uppercase',
    'probe-math-relation-wrap-after',
    'probe-math-relation-wrap-before',
    'probe-math-relation-wrap-repeat',
}
NUMBERING_LAYOUT_CASES = {
    'numbering-alignment-matrix-mac',
    'numbering-legal-format-matrix-mac',
    'numbering-table-header-mac',
    'numbering-table-nested-list-mac',
    'numbering-table-matrix-mac',
    'numbering-textbox-full-matrix-mac',
}
HEADER_TAB_CASES = {'header-tab-overflow-short-mac', 'header-tab-overflow-long-mac'}
FIXED_WIDTH_MATRIX_CASES = {'fixed-table-width-matrix-mac'}
LEGACY_AUTOMATIC_GRID_CASES = {'probe-table-flow-unequal-automatic-grid-start-column-1', 'probe-table-flow-unequal-automatic-grid-start-column-2'}
LEGACY_FIXED_PERCENTAGE_COLUMN_CASES = {'probe-unequal-variant-narrow-first-2-9-fixed', 'probe-unequal-variant-three-24-fixed'}
PREFERRED_GRID_BORDER_CASES |= LEGACY_FIXED_PERCENTAGE_COLUMN_CASES
LEGACY_VISIBLE_AUTOFIT_CASES = {'probe-unequal-variant-narrow-first-2-9-no-header', 'probe-unequal-variant-three-24-no-header', 'probe-unequal-continuation-border-0', 'probe-unequal-continuation-border-16', 'probe-unequal-continuation-border-4'}
BORDERLESS_GRID_CASES = {'probe-unequal-continuation-border-0'}
PREFERRED_GRID_BORDER_CASES |= LEGACY_VISIBLE_AUTOFIT_CASES - BORDERLESS_GRID_CASES
PHYSICAL_FONT_CASES |= PREFERRED_GRID_BORDER_CASES | NUMBERING_LAYOUT_CASES | HEADER_TAB_CASES | FIXED_WIDTH_MATRIX_CASES | LEGACY_AUTOMATIC_GRID_CASES | LEGACY_VISIBLE_AUTOFIT_CASES | LEGACY_FIXED_PERCENTAGE_COLUMN_CASES
MATH_FRACTION_RULE_CASES = {'probe-math-fraction-size-20',
 'probe-math-fraction-size-24',
 'probe-math-fraction-size-28',
 'probe-math-fraction-size-32',
 'probe-math-fraction-size-36',
 'probe-math-fraction-size-40',
 'probe-math-fraction-spacing-matrix',
 'probe-math-repeat-bar-fraction-24',
 'probe-math-repeat-bar-fraction-32',
 'probe-math-standalone-fractions-12',
 'probe-math-standalone-fractions-explicit-sizes'}
BORDERLESS_MATH_CASES = {'probe-math-fraction-first-vs-prev',
 'probe-math-fraction-larger-neighbors',
 'probe-math-fraction-leading',
 'probe-math-fraction-neighbors',
 'probe-math-repeat-fraction-20',
 'probe-math-repeat-fraction-24',
 'probe-math-repeat-fraction-28',
 'probe-math-repeat-fraction-36',
 'probe-math-repeat-fraction-40'}
OVERSIZED_MANUAL_BREAK_CASES = {'table-oversized-row-after-spacing-continuation-fit',
 'table-oversized-row-after-spacing-continuation-overflow',
 'table-oversized-row-after-spacing-fresh-overflow'}
PHYSICAL_FONT_CASES |= OVERSIZED_MANUAL_BREAK_CASES
ADDITIONAL_AFTER_SPACING_CASES = {'nested-after-spacing-double-nested',
 'nested-after-spacing-oversized-49-0',
 'nested-after-spacing-oversized-50-1',
 'nested-after-spacing-short-10pt',
 'table-row-without-headings-mac'}
PHYSICAL_FONT_CASES |= ADDITIONAL_AFTER_SPACING_CASES
ROW_AFTER_SPACING_CASES = {'latin-break-budget-630',
 'latin-row-real-continuation',
 'table-cell-after-spacing-later-paragraph',
 'table-cell-after-spacing-manual-break',
 'table-cell-after-spacing-move-entire-row',
 'table-cell-after-spacing-shift-continuation',
 'table-cell-after-spacing-split-control',
 'table-row-after-spacing-asymmetric',
 'table-row-after-spacing-short-10pt'}
CONTINUED_HEADER_FRAME_CASES = {'probe-table-flow-split-rows-continuous-cell-spacing-start-column-1',
 'probe-table-flow-split-rows-continuous-cell-spacing-start-column-2'}
PHYSICAL_FONT_CASES |= ROW_AFTER_SPACING_CASES | CONTINUED_HEADER_FRAME_CASES
RTL_SPACED_PERCENTAGE_CASES = {'probe-table-flow-unequal-automatic-grid-split-rows-continuous-cell-spacing-three-columns-rtl-section-start-column-1',
 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-cell-spacing-three-columns-rtl-section-start-column-2',
 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-cell-spacing-three-columns-rtl-section-start-column-3'}
RTL_SPACED_PERCENTAGE_EMPTY_RULE_PAGES = {'probe-table-flow-unequal-automatic-grid-split-rows-continuous-cell-spacing-three-columns-rtl-section-start-column-1': (),
 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-cell-spacing-three-columns-rtl-section-start-column-2': (),
 'probe-table-flow-unequal-automatic-grid-split-rows-continuous-cell-spacing-three-columns-rtl-section-start-column-3': (8,)}
PHYSICAL_FONT_CASES |= RTL_SPACED_PERCENTAGE_CASES
INHERITED_RIGHT_MATH_CASES = {'probe-math-display-alignment-0'}
PHYSICAL_FONT_CASES |= INHERITED_RIGHT_MATH_CASES
PARAGRAPH_MARK_MATH_CASES = {'probe-math-paragraph-mark-size'}
PHYSICAL_FONT_CASES |= MATH_FRACTION_RULE_CASES | BORDERLESS_MATH_CASES | PARAGRAPH_MARK_MATH_CASES
RTL_MERGED_PERCENTAGE_CASES = {'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-1',
 'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-2',
 'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-3'}
PHYSICAL_FONT_CASES |= RTL_MERGED_PERCENTAGE_CASES
REQUIRED_CAPTURES = {
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-cell-spacing-three-columns-rtl-section-start-column-1': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-cell-spacing-three-columns-rtl-section-start-column-2': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-cell-spacing-three-columns-rtl-section-start-column-3': '20261114T004450252Z-ac1d5d7d',

    'probe-table-flow-split-rows-continuous-cell-spacing-start-column-1': '20261114T004434269Z-e27596ca',
    'probe-table-flow-split-rows-continuous-cell-spacing-start-column-2': '20261114T004434269Z-e27596ca',
    'table-cell-after-spacing-later-paragraph': '20261114T004633221Z-54077165',
    'table-cell-after-spacing-manual-break': '20261114T004633221Z-54077165',
    'table-cell-after-spacing-move-entire-row': '20261114T004633221Z-54077165',
    'table-cell-after-spacing-shift-continuation': '20261114T004633221Z-54077165',
    'table-cell-after-spacing-split-control': '20261114T004633221Z-54077165',
    'table-row-after-spacing-asymmetric': '20261114T020856500Z-ff0457a1',
    'table-row-after-spacing-short-10pt': '20261114T004647940Z-877c5941',
    'latin-break-budget-630': '20261114T004309406Z-2c388f57',
    'latin-row-real-continuation': '20261114T004322231Z-e7a4d03f',

    'nested-after-spacing-double-nested': '20261114T004322231Z-e7a4d03f',
    'nested-after-spacing-oversized-49-0': '20261114T004322231Z-e7a4d03f',
    'nested-after-spacing-oversized-50-1': '20261114T004322231Z-e7a4d03f',
    'nested-after-spacing-short-10pt': '20261114T004334255Z-b74a0cd3',
    'table-row-without-headings-mac': '20261114T004647940Z-877c5941',

    'table-oversized-row-after-spacing-continuation-fit': '20261114T020830750Z-13e77d19',
    'table-oversized-row-after-spacing-continuation-overflow': '20261114T020832449Z-7b8b0314',
    'table-oversized-row-after-spacing-fresh-overflow': '20261114T020840914Z-686154c0',

    'probe-math-display-alignment-0': '20261114T004747969Z-36728554',
    'probe-math-paragraph-mark-size': '20261114T004417585Z-59a68986',
    'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-1': '20261114T020625597Z-2761cea8',
    'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-2': '20261114T020627365Z-1088d320',
    'probe-table-flow-unequal-continuous-vertical-merge-three-columns-rtl-section-start-column-3': '20261114T020631298Z-7877fbf1',

    'probe-math-fraction-first-vs-prev': '20261114T004417585Z-59a68986',
    'probe-math-fraction-larger-neighbors': '20261114T004417585Z-59a68986',
    'probe-math-fraction-leading': '20261114T004417585Z-59a68986',
    'probe-math-fraction-neighbors': '20261114T004417585Z-59a68986',
    'probe-math-fraction-size-20': '20261114T004417585Z-59a68986',
    'probe-math-fraction-size-24': '20261114T004417585Z-59a68986',
    'probe-math-fraction-size-28': '20261114T004417585Z-59a68986',
    'probe-math-fraction-size-32': '20261114T004417585Z-59a68986',
    'probe-math-fraction-size-36': '20261114T004417585Z-59a68986',
    'probe-math-fraction-size-40': '20261114T004417585Z-59a68986',
    'probe-math-fraction-spacing-matrix': '20261114T004417585Z-59a68986',
    'probe-math-repeat-bar-fraction-24': '20261114T004417585Z-59a68986',
    'probe-math-repeat-bar-fraction-32': '20261114T004417585Z-59a68986',
    'probe-math-repeat-fraction-20': '20261114T004417585Z-59a68986',
    'probe-math-repeat-fraction-24': '20261114T004434269Z-e27596ca',
    'probe-math-repeat-fraction-28': '20261114T004434269Z-e27596ca',
    'probe-math-repeat-fraction-36': '20261114T004434269Z-e27596ca',
    'probe-math-repeat-fraction-40': '20261114T004434269Z-e27596ca',
    'probe-math-standalone-fractions-12': '20261114T004434269Z-e27596ca',
    'probe-math-standalone-fractions-explicit-sizes': '20261114T004434269Z-e27596ca',

    'probe-unequal-variant-narrow-first-2-9-no-header': '20261114T004559814Z-c5e13cee',
    'probe-unequal-variant-three-24-no-header': '20261114T004559814Z-c5e13cee',
    'probe-unequal-continuation-border-0': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-continuation-border-16': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-continuation-border-4': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-variant-narrow-first-2-9-fixed': '20261114T004559814Z-c5e13cee',
    'probe-unequal-variant-three-24-fixed': '20261114T004559814Z-c5e13cee',
    'probe-table-flow-unequal-automatic-grid-start-column-1': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-start-column-2': '20261114T004450252Z-ac1d5d7d',
    'probe-mixed-sangam-matrix-240': '20261114T004434269Z-e27596ca',
    'probe-mixed-sangam-matrix-276': '20261114T004434269Z-e27596ca',

    'probe-devanagari-line-box-devanagari': '20261114T004402030Z-30b8d3b5',
    'probe-complex-font-sweep-thonburi': '20261114T004402030Z-30b8d3b5',
    'probe-complex-font-metrics-thonburi': '20261114T004402030Z-30b8d3b5',
    'courier-new-line-metrics': '20261114T004252975Z-a999e8fb',
    'courier-new-regular-adjacent': '20261114T004252975Z-a999e8fb',
    'probe-column-fixed-dxa-4400': '20261114T004402030Z-30b8d3b5',
    'tabs-geometry-mac': '20261114T130556208Z-6d6ad7b2',
    '2col-mac': '20261114T231659226Z-87aed8b1',
    'char-spacing-mac': '20261114T231718277Z-f02fe908',
    'word-compat-comprehensive-mac': '20261114T004202539Z-7cf29738',
    'floating-inside-outside-mac': '20261114T004143028Z-b4cb9887',
    'header-footer-edge-cases-mac': '20261114T004143028Z-b4cb9887',
    'textbox-body-properties-mac': '20261114T004202539Z-7cf29738',
    'textbox-tall-header-overlap-mac': '20261114T004202539Z-7cf29738',
    'toc-instrtext-edge-cases-mac': '20261114T004202539Z-7cf29738',
    'floating-tight-polygon-mac': '20261114T004143028Z-b4cb9887',
    'word-mac-probe-float-lookahead': '20261114T004202539Z-7cf29738',
    'header-footer-sectionpages-mac': '20261114T004143028Z-b4cb9887',
    'word-mac-probe-floating-horizontal-14': '20261114T004202539Z-7cf29738',
    'word-mac-probe-grid-line-unit': '20261114T004214564Z-4ebc9049',
    'word-mac-probe-grid-line-unit-table': '20261114T004214564Z-4ebc9049',
    'word-mac-probe-pairs-kerned': '20261114T004214564Z-4ebc9049',
    'borderless-table-mac': '20261114T004252975Z-a999e8fb',
    'exact-and-minimum-lines-mac': '20261114T020444292Z-63287be8',
    'aptos-kerning-mac': '20261114T004240145Z-5f9102be',
    'aptos-numeric-ranges-mac': '20261114T004240145Z-5f9102be',
    'aptos-range-compat-15-en-US-mac': '20261114T004240145Z-5f9102be',
    'column-table-fixed-preferred-width-mac': '20261114T004559814Z-c5e13cee',
    'column-table-fixed-preferred-width-smaller-mac': '20261114T004346218Z-5f779447',
    'aptos-ascii-advances-mac': '20261117T123854554Z-f62dd6d1',
    'floating-square-wrap-mac': '20261114T004143028Z-b4cb9887',
    'floating-wrap-sides-mac': '20261114T004143028Z-b4cb9887',
    'word-mac-probe-floating-horizontal-15': '20261114T015622405Z-203d1791',
    'probe-hyphenation-split-word-sweep-off': '20261114T020157033Z-88c4d52e',
    'probe-contextual-paragraph-spacing-mode14': '20261114T023153447Z-6c24de77',
    'probe-contextual-paragraph-spacing-mode15': '20261114T023156058Z-d477ae1f',
    'probe-paragraph-border-wrap': '20261114T023201516Z-623785ec',
    'endnote-mac': '20261114T004143028Z-b4cb9887',
    'footnote-long-continuation-mac': '20261114T004143028Z-b4cb9887',
    'footnote-table-textbox-order-mac': '20261114T004143028Z-b4cb9887',
    'footnote-mac': '20261114T004143028Z-b4cb9887',
    'footnote-table-each-page-mac': '20261114T004143028Z-b4cb9887',
    'align-center-mac': '20261114T020438767Z-bfed5b87',
    'alignment-mac': '20261114T020441411Z-af6ea560',
    'aptos-range-compat-14-en-US-mac': '20261114T004240145Z-5f9102be',
    'ascii-punctuation-mac': '20261114T004252975Z-a999e8fb',
    'cell-alignment-mac': '20261114T004252975Z-a999e8fb',
    'column-break-mac': '20261114T004252975Z-a999e8fb',
    'decimal-tabs-mac': '20261114T004252975Z-a999e8fb',
    'direction-with-selection-mac': '20261114T004143028Z-b4cb9887',
    'eqruby-grid-mac': '20261114T004202539Z-7cf29738',
    'eqruby-nogrid-mac': '20261114T004202539Z-7cf29738',
    'fractional-picture-anchors-mac': '20261114T020548688Z-7207704a',
    'header-footer-sections-mac': '20261114T004143028Z-b4cb9887',
    'header-tab-overflow-long-mac': '20261114T020253083Z-9dcfef94',
    'indents-mac': '20261114T004309406Z-2c388f57',
    'inline-image-position-grid-mac': '20261114T004813814Z-c21deb91',
    'inline-image-position-nogrid-mac': '20261114T004813814Z-c21deb91',
    'inline-image-rotation-table-picture-only-mac': '20261114T004309406Z-2c388f57',
    'jade-script-baseline-20261030': '20261114T004704886Z-7e8168a7',
    'kana-mac': '20261114T004309406Z-2c388f57',
    'keep-mac': '20261114T004309406Z-2c388f57',
    'latin-break-budget-620': '20261114T004309406Z-2c388f57',
    'latin-break-budget-638': '20261114T004309406Z-2c388f57',
    'latin-break-budget-645': '20261114T004309406Z-2c388f57',
    'latin-break-budget-652': '20261114T004309406Z-2c388f57',
    'latin-break-budget-659': '20261114T004309406Z-2c388f57',
    'line-numbering-mac': '20261114T004322231Z-e7a4d03f',
    'line-spacing-mac': '20261114T004322231Z-e7a4d03f',
    'merged-table-over-page-mac': '20261114T005709592Z-772bee19',
    'mirror-margins-mac': '20261114T004322231Z-e7a4d03f',
    'mirror-margins-parity-mac': '20261114T020311702Z-0fc768f7',
    'nested-after-spacing-oversized-48-0': '20261114T004322231Z-e7a4d03f',
    'nested-after-spacing-repeat-header': '20261114T004322231Z-e7a4d03f',
    'nested-after-spacing-short-5pt': '20261114T004334255Z-b74a0cd3',
    'nested-merged-tables-mac': '20261114T004334255Z-b74a0cd3',
    'note-styles-creation-mac': '20261114T020313084Z-50588e4b',
    'numbering-by-style-mac': '20261114T020314478Z-267eae10',
    'numbering-restart-mac': '20261114T020324309Z-57157ab7',
    'odd-even-stories-mac': '20261114T004334255Z-b74a0cd3',
    'paragraph-spacing-mac': '20261114T004346218Z-5f779447',
    'probe-unequal-left-narrow-first-1-1': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-left-narrow-first-2-1': '20261114T004547596Z-7db3b26c',
    'probe-unequal-left-wide-first-1-1': '20261114T004547596Z-7db3b26c',
    'probe-unequal-left-wide-first-2-1': '20261114T004547596Z-7db3b26c',
    'ruby-grid-mac': '20261114T004214564Z-4ebc9049',
    'ruby-nogrid-mac': '20261114T004214564Z-4ebc9049',
    'ruby-wide-grid-mac': '20261114T004214564Z-4ebc9049',
    'ruby-wide-nogrid-mac': '20261114T004214564Z-4ebc9049',
    'run-styles-mac': '20261114T020737875Z-7d16155a',
    'script-position-mac': '20261114T020739586Z-407e2e78',
    'section-break-types-mac': '20261114T021536775Z-818619fb',
    'sections-mac': '20261114T020750373Z-ad48fa89',
    'story-inline-picture-metrics-mac': '20261114T020753623Z-4d5f0b2b',
    'table-boundary-cant-split-mac': '20261114T004633221Z-54077165',
    'table-oversized-row-after-spacing-fresh-fit': '20261114T020835626Z-17c9262c',
    'table-row-after-spacing-short-5pt': '20261114T004647940Z-877c5941',
    'table-vertical-merge-page-break-mac': '20261114T004202539Z-7cf29738',
    'table3x2merged-mac': '20261114T004647940Z-877c5941',
    'tabs-mac': '20261114T004647940Z-877c5941',
    'textbox-exact-line-matrix-mac': '20261114T004647940Z-877c5941',
    'textbox-fixed-pitch-matrix-mac': '20261114T004647940Z-877c5941',
    'toc-sequence-page-number-mac': '20261114T004202539Z-7cf29738',
    'underline-kinds-mac': '20261114T004202539Z-7cf29738',
    'unequal-columns-mac': '20261114T004647940Z-877c5941',
    'vert-align-sweep-2-browser-mac': '20261114T020406409Z-43ea3b48',
    'vert-align-sweep-browser-mac': '20261114T004647940Z-877c5941',
    'widow-control-mac': '20261114T004704886Z-7e8168a7',
    'word-mac-probe-distribute-single': '20261114T004202539Z-7cf29738',
    'word-mac-probe-indent': '20261114T004214564Z-4ebc9049',
    'word-mac-probe-numbering-label': '20261114T004214564Z-4ebc9049',
    'word-mac-probe-numbering-label-faces': '20261114T004214564Z-4ebc9049',
    'word-mac-probe-object-space-runs-unbalanced': '20261114T004214564Z-4ebc9049',
    'word-mac-probe-squeeze-conditions-nocompress': '20261114T020348514Z-150cc80a',
    'word-mac-probe-squeeze-plain': '20261114T020357360Z-c1ab8e03',
    'word-mac-probe-vert-align-sweep-2': '20261114T004704886Z-7e8168a7',
    'wrong-num-mac': '20261114T020410811Z-30c286b3',
    'floating-simple-position-mac': '20261114T004143028Z-b4cb9887',
    'floating-through-concave-mac': '20261114T004143028Z-b4cb9887',
    'floating-z-order-mac': '20261114T004143028Z-b4cb9887',
    'floating-top-bottom-offset-mac': '20261114T004143028Z-b4cb9887',
    'probe-column-fixed-dxa-6800': '20261114T004402030Z-30b8d3b5',
    'probe-width-single-dxa-2400-dxa-2000': '20261114T004612020Z-5d981186',
    'probe-continuous-column-explicit-break': '20261114T004252975Z-a999e8fb',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-1': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-2': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-1800-twips-start-column-3': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-1': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-2': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2160-twips-start-column-3': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-1': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-2': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-header-2460-twips-start-column-3': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-1': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-2': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-start-column-3': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-1': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-2': '20261114T004450252Z-ac1d5d7d',
    'probe-table-flow-unequal-automatic-grid-split-rows-continuous-right-three-columns-rtl-section-tall-header-start-column-3': '20261114T004450252Z-ac1d5d7d',
    'probe-fixed-auto-table-grid-2000-cell-pct-1250': '20261114T004402030Z-30b8d3b5',
    'probe-fixed-auto-table-grid-1000-cell-pct-2500': '20261114T004402030Z-30b8d3b5',
    'probe-fixed-auto-table-grid-2000-cell-pct-5000': '20261114T004402030Z-30b8d3b5',
    'probe-fixed-auto-table-grid-4000-cell-pct-2500': '20261114T004402030Z-30b8d3b5',
    'probe-unequal-variant-narrow-first-2-9-dxa': '20261114T004559814Z-c5e13cee',
    'probe-width-equal-auto-0-pct-2500': '20261114T004559814Z-c5e13cee',
    'probe-width-single-auto-0-pct-2500': '20261114T004612020Z-5d981186',
    'probe-unequal-autofit-wrapped-header': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-center-narrow-first-2-9': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-center-three-10': '20261114T004519313Z-65bdf3a6',
    'probe-table-flow-unequal-start-column-1': '20261114T020647499Z-11e18e84',
    'probe-table-flow-unequal-start-column-2': '20261114T020650547Z-40629d69',
    'probe-unequal-autofit-section-width-matrix': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-center-narrow-first-1-1': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-center-narrow-first-1-9': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-center-three-13': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-center-three-17': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-center-three-24': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-center-three-4': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-center-wide-first-1-1': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-center-wide-first-1-9': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-center-wide-first-2-9': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-control-1-16-with-tail': '20261114T004519313Z-65bdf3a6',
    'probe-unequal-control-1-16-without-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-1-21-with-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-1-21-without-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-1-22-with-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-1-22-without-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-1-23-with-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-1-23-without-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-1-24-without-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-2-12-with-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-2-12-without-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-2-19-with-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-2-19-without-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-3-14-with-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-3-14-without-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-3-8-with-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-control-3-8-without-tail': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-explicit-paragraph-width-3300': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-explicit-paragraph-width-3566': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-explicit-paragraph-width-3800': '20261114T004533252Z-4b33f3f3',
    'probe-unequal-variant-narrow-first-2-9-center': '20261114T004547596Z-7db3b26c',
    'probe-unequal-variant-narrow-first-2-9-long': '20261114T004559814Z-c5e13cee',
    'probe-unequal-variant-three-24-center': '20261114T004559814Z-c5e13cee',
    'probe-unequal-variant-three-24-long': '20261114T004559814Z-c5e13cee',
    'probe-width-short-narrow-first-1-1': '20261114T004612020Z-5d981186',
    'probe-width-short-narrow-first-1-4': '20261114T004612020Z-5d981186',
    'probe-width-short-narrow-first-1-5': '20261114T004612020Z-5d981186',
    'probe-width-short-narrow-first-1-9': '20261114T004612020Z-5d981186',
    'probe-width-short-narrow-first-2-1': '20261114T004612020Z-5d981186',
    'probe-width-short-narrow-first-2-4': '20261114T004612020Z-5d981186',
    'probe-width-short-narrow-first-2-5': '20261114T004612020Z-5d981186',
    'probe-width-short-narrow-first-2-9': '20261114T004612020Z-5d981186',
    'probe-width-short-wide-first-1-1': '20261114T004612020Z-5d981186',
    'probe-width-short-wide-first-1-4': '20261114T004612020Z-5d981186',
    'probe-width-short-wide-first-1-5': '20261114T004612020Z-5d981186',
    'probe-width-short-wide-first-1-9': '20261114T004612020Z-5d981186',
    'probe-width-short-wide-first-2-1': '20261114T004612020Z-5d981186',
    'probe-width-short-wide-first-2-4': '20261114T004612020Z-5d981186',
    'probe-width-short-wide-first-2-5': '20261114T004612020Z-5d981186',
    'probe-width-short-wide-first-2-9': '20261114T004612020Z-5d981186',
    'probe-width-unequal-auto-0-pct-2500': '20261114T004612020Z-5d981186',
    'probe-width-unequal-pct-2500-pct-2500': '20261114T020703561Z-b1e8f880',
    'probe-width-unequal-pct-2500-pct-5000': '20261114T020706597Z-592f23c9',
    'probe-width-unequal-pct-5000-pct-2500': '20261114T020709734Z-0f2de737',
    'probe-unequal-variant-three-24-left': '20261114T004559814Z-c5e13cee',
    'probe-unequal-variant-three-24-right': '20261114T004559814Z-c5e13cee',
    'probe-second-column-centered-table': '20261114T004434269Z-e27596ca',
    'probe-table-flow-cell-spacing-start-column-1': '20261114T004434269Z-e27596ca',
    'probe-table-flow-continuous-start-column-1': '20261114T020756625Z-d80a345e',
    'probe-table-flow-continuous-start-column-2': '20261114T004434269Z-e27596ca',
    'probe-width-equal-pct-2500-auto-0': '20261114T004559814Z-c5e13cee',
    'probe-width-equal-pct-2500-dxa-2000': '20261114T004559814Z-c5e13cee',
    'probe-width-equal-pct-2500-pct-2500': '20261114T004559814Z-c5e13cee',
    'probe-width-equal-pct-2500-pct-5000': '20261114T004559814Z-c5e13cee',
    'probe-width-equal-pct-5000-pct-2500': '20261114T004559814Z-c5e13cee',
    'probe-table-flow-continuous-vertical-merge-start-column-1': '20261114T004434269Z-e27596ca',
    'probe-table-flow-continuous-vertical-merge-start-column-2': '20261114T020800482Z-949d78a3',
    'probe-table-flow-split-rows-continuous-start-column-1': '20261114T004434269Z-e27596ca',
    'probe-table-flow-split-rows-start-column-1': '20261114T020802205Z-aa2e15c3',
    'probe-table-flow-split-rows-start-column-2': '20261114T004434269Z-e27596ca',
    'probe-table-flow-start-column-1': '20261114T004434269Z-e27596ca',
    'probe-table-flow-start-column-2': '20261114T004434269Z-e27596ca',
    'table-flow-continuous-start-column-1-mac': '20261114T020756625Z-d80a345e',
    'table-flow-continuous-vertical-merge-start-column-2-mac': '20261114T020800482Z-949d78a3',
    'table-flow-split-rows-start-column-1-mac': '20261114T020802205Z-aa2e15c3',
    'probe-unequal-variant-three-24-dxa': '20261114T004559814Z-c5e13cee',
    'table-bidi-alignment-mac': '20261114T004633221Z-54077165',
    'table-bidi-visual-mac': '20261114T004633221Z-54077165',
    'table-border-conflicts-mac': '20261114T004633221Z-54077165',
    'table-diagonal-borders-mac': '20261114T004633221Z-54077165',
    'table-hidden-rows-mac': '20261114T020336297Z-3b4ec000',
    'table-hide-mark-mac': '20261114T020337962Z-d9270837',
    'table-repeating-header-mac': '20261114T020853056Z-39d2496c',
    'probe-table-flow-unequal-continuous-right-three-columns-start-column-1': '20261114T020617782Z-16fdc6f5',
    'probe-table-flow-unequal-continuous-right-three-columns-start-column-2': '20261114T020620874Z-41445fa5',
    'probe-table-flow-unequal-continuous-right-three-columns-start-column-3': '20261114T020623944Z-655e03c7',
    'probe-table-flow-unequal-right-start-column-1': '20261114T020634389Z-b8cad4d3',
    'probe-table-flow-unequal-right-start-column-2': '20261114T020637436Z-52053326',
    'probe-unequal-left-narrow-first-1-9': '20261114T004547596Z-7db3b26c',
    'probe-unequal-left-narrow-first-2-9': '20261114T004547596Z-7db3b26c',
    'probe-unequal-left-start-column-1': '20261114T004547596Z-7db3b26c',
    'probe-unequal-left-wide-first-1-9': '20261114T004547596Z-7db3b26c',
    'probe-unequal-left-wide-first-2-9': '20261114T004547596Z-7db3b26c',
    'probe-unequal-right-narrow-first-1-1': '20261114T004547596Z-7db3b26c',
    'probe-unequal-right-narrow-first-1-9': '20261114T004547596Z-7db3b26c',
    'probe-unequal-right-narrow-first-2-9': '20261114T004547596Z-7db3b26c',
    'probe-unequal-right-three-10': '20261114T004547596Z-7db3b26c',
    'probe-unequal-right-three-17': '20261114T004547596Z-7db3b26c',
    'probe-unequal-right-three-24': '20261114T004547596Z-7db3b26c',
    'probe-unequal-right-three-4': '20261114T004547596Z-7db3b26c',
    'probe-unequal-right-three-7': '20261114T004547596Z-7db3b26c',
    'probe-unequal-right-wide-first-1-1': '20261114T004547596Z-7db3b26c',
    'probe-unequal-right-wide-first-1-9': '20261114T004547596Z-7db3b26c',
    'probe-unequal-right-wide-first-2-9': '20261114T004547596Z-7db3b26c',
    'probe-unequal-variant-narrow-first-2-9-left': '20261114T004559814Z-c5e13cee',
    'probe-unequal-variant-narrow-first-2-9-right': '20261114T004559814Z-c5e13cee',
    'table-legacy-hmerge-mac': '20261114T020815305Z-ab0cdcfa',
    'table-cell-fit-text-mac': '20261114T004633221Z-54077165',
    'probe-math-display-alignment-1': '20261114T004747969Z-36728554',
    'probe-math-display-rtl-wrap-1': '20261114T004747969Z-36728554',
    'probe-math-italic-advances-lowercase': '20261114T004417585Z-59a68986',
    'probe-math-italic-advances-uppercase': '20261114T004417585Z-59a68986',
    'probe-math-relation-wrap-after': '20261114T004417585Z-59a68986',
    'probe-math-relation-wrap-before': '20261114T004417585Z-59a68986',
    'probe-math-relation-wrap-repeat': '20261114T004417585Z-59a68986',

    'probe-width-single-pct-2500-pct-2500': '20261114T004612020Z-5d981186',
    'probe-width-unequal-dxa-2400-dxa-2000': '20261114T020653579Z-a7f517aa',
    'probe-width-unequal-pct-2500-auto-0': '20261114T020656594Z-92d3b2ca',
    'probe-width-unequal-pct-2500-dxa-2000': '20261114T020700519Z-4ca7ac7c',

    'numbering-alignment-matrix-mac': '20261114T004334255Z-b74a0cd3',
    'numbering-legal-format-matrix-mac': '20261114T004334255Z-b74a0cd3',
    'numbering-textbox-full-matrix-mac': '20261114T004334255Z-b74a0cd3',

    'numbering-table-header-mac': '20261114T004334255Z-b74a0cd3',
    'numbering-table-nested-list-mac': '20261114T004334255Z-b74a0cd3',
    'numbering-table-matrix-mac': '20261114T004334255Z-b74a0cd3',

    'header-tab-overflow-short-mac': '20261114T020256534Z-0729bfd7',
    'fixed-table-width-matrix-mac': '20261114T020447264Z-b8062685',
}

# These original inputs have distinct screen and print origins in Windows
# Word. Their live layout must match EMF; their actual vector PDF must match
# Word's PDF. Existing shared screen/PDF cases retain all four assertions.
INDEPENDENT_OUTPUT_CASES = frozenset({
    'word-compat-comprehensive-mac',
    'floating-inside-outside-mac',
    'header-footer-edge-cases-mac',
    'textbox-body-properties-mac',
    'textbox-tall-header-overlap-mac',
    'toc-instrtext-edge-cases-mac',
    'floating-tight-polygon-mac',
    'header-footer-sectionpages-mac',
    'floating-simple-position-mac',
    'floating-through-concave-mac',
    'floating-z-order-mac',
    'floating-top-bottom-offset-mac',
    'table-bidi-alignment-mac',
    'table-bidi-visual-mac',
    'table-border-conflicts-mac',
    'table-diagonal-borders-mac',
    'table-hidden-rows-mac',
    'table-hide-mark-mac',
    'table-repeating-header-mac',
    'table-legacy-hmerge-mac',
    'table-cell-fit-text-mac',
})


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read_json(path):
    return json.loads(path.read_text(encoding='utf-8-sig'))


def repo_file(root, relative):
    require(isinstance(relative, str) and '\\' not in relative and ':' not in relative
            and not Path(relative).is_absolute(),
            'Expected repository-relative POSIX path')
    path = (root / relative).resolve()
    require(path.is_relative_to(root.resolve()), 'Path escapes repository')
    require(path.is_file(), f'Missing evidence: {relative}')
    return path


def checked_file(root, record):
    path = repo_file(root, record['file'])
    require(hashlib.sha256(path.read_bytes()).hexdigest() == record['sha256'],
            f'Evidence changed: {record["file"]}')
    return path


def ordered(glyphs):
    result = sorted((g for g in glyphs if not g[1].isspace()), key=lambda g: (g[0], g[3]))
    start = 0
    while start < len(result):
        end = start + 1
        while (end < len(result) and result[end][0] == result[start][0]
               and abs(result[end][3] - result[start][3]) <= .25):
            end += 1
        result[start:end] = sorted(result[start:end], key=lambda g: g[2])
        start = end
    return result


def compare(expected, actual, label, *, font_metrics=False):
    require(actual['page_count'] == expected['page_count'], f'{label}: page count')
    expected_glyphs = [g for g in expected['glyphs'] if not g[1].isspace()]
    actual_glyphs = [g for g in actual['glyphs'] if not g[1].isspace()]
    require(bool(expected_glyphs), f'{label}: empty reference')
    require(len(expected_glyphs) == len(actual_glyphs), f'{label}: missing/extra glyphs')
    require(Counter(tuple(g[:2]) for g in expected_glyphs)
            == Counter(tuple(g[:2]) for g in actual_glyphs),
            f'{label}: text or page changed')
    # Floating text and a superscript can be only 0.24pt apart vertically.
    # Grouping baselines at 0.25pt changes reading order after harmless Word
    # rounding. Match the same character/page at its actual XY origin instead;
    # every origin still needs a distinct partner within 0.375pt on BOTH axes.
    tolerance = .375
    if font_metrics:
        for model in (expected_glyphs, actual_glyphs):
            require(all(len(g) == 6 and isinstance(g[4], str) and bool(g[4])
                        and math.isfinite(g[5]) and g[5] > 0 for g in model),
                    f'{label}: invalid font metrics')
    buckets = defaultdict(list)
    for i, glyph in enumerate(actual_glyphs):
        require(all(math.isfinite(glyph[axis]) for axis in (2, 3)),
                f'{label}/{i}: nonfinite axis')
        buckets[(glyph[0], glyph[1], math.floor(glyph[2] / tolerance),
                 math.floor(glyph[3] / tolerance))].append(i)
    candidates = []
    for i, word in enumerate(expected_glyphs):
        x, y = math.floor(word[2] / tolerance), math.floor(word[3] / tolerance)
        nearby = [j for dx in (-1, 0, 1) for dy in (-1, 0, 1)
                  for j in buckets.get((word[0], word[1], x + dx, y + dy), ())
                  if all(abs(word[axis] - actual_glyphs[j][axis]) <= tolerance
                         for axis in (2, 3))
                  and (not font_metrics or (word[4] == actual_glyphs[j][4]
                       and abs(word[5] - actual_glyphs[j][5]) <= .001))]
        require(bool(nearby), f'{label}/{i}: axis mismatch: Word {word}')
        nearby.sort(key=lambda j: sum((word[axis] - actual_glyphs[j][axis]) ** 2
                                      for axis in (2, 3)))
        candidates.append(nearby)
    word_match, jade_match = {}, {}
    for start in sorted(range(len(candidates)), key=lambda i: len(candidates[i])):
        queue, seen, predecessor = deque([start]), {start}, {}
        free = None
        while queue and free is None:
            word_index = queue.popleft()
            for jade_index in candidates[word_index]:
                if jade_index in predecessor:
                    continue
                predecessor[jade_index] = word_index
                if jade_index not in jade_match:
                    free = jade_index
                    break
                other = jade_match[jade_index]
                if other not in seen:
                    seen.add(other)
                    queue.append(other)
        require(free is not None, f'{label}/{start}: missing distinct glyph at expected axes')
        while free is not None:
            word_index = predecessor[free]
            previous = word_match.get(word_index)
            word_match[word_index], jade_match[free] = free, word_index
            free = previous
    return len(expected_glyphs)


def pdf_font_program_aliases(document, page, cache):
    """Resolve PDF display aliases only from that PDF's embedded font names."""
    import io
    from fontTools.ttLib import TTFont
    aliases = defaultdict(set)
    for font in page.get_fonts(full=True):
        xref, _, kind, declared = font[:4]
        if kind == 'Type3':
            continue
        if xref not in cache:
            _, extension, _, data = document.extract_font(xref)
            identity = None
            if data and extension in ('ttf', 'otf'):
                with TTFont(io.BytesIO(data)) as program:
                    values = defaultdict(set)
                    if 'name' in program:
                        for record in program['name'].names:
                            if record.nameID in (1, 2, 4, 6, 16, 17):
                                value = re.sub(r'^[A-Z]{6}\+', '', record.toUnicode()).strip()
                                if value:
                                    values[record.nameID].add(value)
                    postscript = values[6]
                    require(len(postscript) <= 1, 'Ambiguous embedded PostScript font name')
                    if postscript:
                        actual = next(iter(postscript))
                        permitted = set(postscript) | values[4]
                        for family in values[1] | values[16]:
                            for style in values[2] | values[17]:
                                if style.casefold() in ('regular', 'normal', 'roman'):
                                    permitted.add(family)
                                else:
                                    permitted.update((family + ',' + style.replace(' ', ''),
                                                      family + ' ' + style))
                        identity = (actual, permitted)
            cache[xref] = identity
        identity = cache[xref]
        alias = re.sub(r'^[A-Z]{6}\+', '', declared)
        if identity is not None and alias in identity[1]:
            aliases[alias].add(identity[0])
    require(all(len(values) == 1 for values in aliases.values()),
            'Ambiguous PDF font alias')
    return {alias: next(iter(values)) for alias, values in aliases.items()}


def pdf_font_identity(document, span_font, aliases=None):
    """Read the actual PDF face, including unnamed Type3 outline fonts."""
    if span_font == 'Type3' or span_font.startswith('Type3 ('):
        match = re.fullmatch(r'Type3 \((\d+) 0 R\)', span_font)
        require(match is not None, 'Type3 font identity lacks an object reference')
        xref = int(match.group(1))
        require(0 < xref < document.xref_length()
                and document.xref_get_key(xref, 'Subtype') == ('name', '/Type3'),
                'Invalid Type3 font reference')
        kind, declared = document.xref_get_key(xref, 'FontDescriptor/FontName')
        require(kind == 'name' and declared.startswith('/') and len(declared) > 1,
                'Type3 font descriptor lacks an actual font name')
        span_font = re.sub(r'#([0-9a-fA-F]{2})',
                           lambda m: chr(int(m.group(1), 16)), declared[1:])
        return re.sub(r'^[A-Z]{6}\+', '', span_font)
    identity = re.sub(r'^[A-Z]{6}\+', '', span_font)
    return (aliases or {}).get(identity, identity)


def pdf_origins(path, unclipped=False, *, font_metrics=False):
    import pymupdf
    flags = pymupdf.TEXTFLAGS_RAWDICT
    if unclipped:
        flags &= ~pymupdf.TEXT_CLIP
    with pymupdf.open(path) as document:
        glyphs = []
        font_cache = {}
        for number, page in enumerate(document, 1):
            aliases = pdf_font_program_aliases(document, page, font_cache) if font_metrics else {}
            for block in page.get_text('rawdict', flags=flags)['blocks']:
                for line in block.get('lines', []):
                    for span in line['spans']:
                        metrics = ([pdf_font_identity(document, span['font'], aliases), span['size']]
                                   if font_metrics else [])
                        glyphs.extend([number, char['c'], *char['origin'], *metrics]
                                      for char in span['chars'] if not char['c'].isspace())
        return dict(page_count=len(document), glyphs=glyphs)


def assert_pending_pdf_text(expected, actual, label):
    """Keep text and pages valid while coordinate mismatches remain pending."""
    require(expected['page_count'] == actual['page_count'], label + ': page count changed')
    require(all(math.isfinite(g[axis]) for g in actual['glyphs'] for axis in (2, 3)),
            label + ': nonfinite glyph origin')
    words = [g[:2] for g in expected['glyphs'] if not g[1].isspace()]
    painted = [g[:2] for g in actual['glyphs'] if not g[1].isspace()]
    require(words == painted and bool(words), label + ': text/page/order changed')



def extract_fonts(pdf, directory):
    """Expose original font programs, never Word's expected text positions."""
    import pymupdf
    directory.mkdir(parents=True, exist_ok=False)
    with pymupdf.open(pdf) as document:
        xrefs = {font[0] for page in document for font in page.get_fonts(full=True)}
        count = 0
        for xref in sorted(xrefs):
            _, extension, _, data = document.extract_font(xref)
            if data and extension in ('ttf', 'otf', 'ttc'):
                (directory / f'{xref}.{extension}').write_bytes(data)
                count += 1
    require(count > 0, 'Original Word PDF contains no usable embedded fonts')


def archive_reference(root, case, windows):
    """Read one document from an immutable, possibly multi-document capture.

    Drawing origins are decoded from original EMFs in memory. The archive and
    capture metadata remain byte-for-byte original; no selected capture or
    renderer-derived expected positions are generated.
    """
    archive = checked_file(root, windows['archive'])
    with ZipFile(archive) as bundle:
        members = bundle.namelist()
        require(len(members) == len(set(members)) and all(
            not Path(name).is_absolute() and ':' not in name and '\\' not in name
            and '..' not in Path(name).parts for name in members),
            'Invalid or duplicate original archive member')
        raw = bundle.read('capture.json')
        require(hashlib.sha256(raw).hexdigest() == windows['capture_sha256'],
                'Original archived capture metadata changed')
        capture = json.loads(raw.decode('utf-8-sig'))
        require(capture['schema'] == 2 and capture['platform'] == 'windows'
                and capture['status'] == 'complete', 'Incomplete or mislabelled Windows capture')
        require(capture['capture_id'] == windows['capture_id'], 'Archived capture identity changed')
        require(capture['word']['product_version'] == windows['word_product_version'],
                'Windows Word version changed')
        require(capture['word']['executable_sha256'] == windows['word_executable_sha256'],
                'Windows Word executable changed')
        require(archive.parent.name == 'word-' + windows['word_product_version'],
                'Wrong archived Word version directory')
        documents = [d for d in capture['documents'] if d['name'] == windows['fixture']]
        require(len(documents) == 1, 'Missing or duplicate archived Windows input')
        document = documents[0]
        # Older captures predate this field. Keep their missing provenance;
        # never relabel them as a newer capture or accept a recorded repair.
        require(document.get('open_mode') in (None, 'normal-read-only'),
                'Archived input was repaired or converted')
        require(document['sha256'] == case['source']['sha256'], 'Windows source identity changed')
        source = checked_file(root, case['source'])
        require(bundle.read('inputs/' + document['name'] + '.docx') == source.read_bytes(),
                'Windows input differs from Mac original')
        require(hashlib.sha256(bundle.read(document['pdf'])).hexdigest() == document['pdf_sha256'],
                'Original archived Windows Word PDF changed')
        pages = document['metafiles']
        require([p['page'] for p in pages] == list(range(1, document['pages'] + 1)),
                'Archived Word page coverage changed')
        glyph_resolver = None
        if 'glyph_index_fonts' in windows:
            from windows_emf_font_maps import original_pdf_glyph_index_resolver
            identity = read_json(checked_file(root, windows['glyph_index_fonts']))
            glyph_resolver = original_pdf_glyph_index_resolver(
                bundle.read(document['pdf']), identity,
                source_sha256=document['sha256'], capture_sha256=windows['capture_sha256'],
                word=capture['word'])
        glyphs = []
        for page in pages:
            data = bundle.read(page['file'])
            require(hashlib.sha256(data).hexdigest() == page['sha256'],
                    'Original archived Windows Word EMF changed')
            glyphs.extend(emf_glyphs(data, page['page'], page['width_pt'], page['height_pt'],
                                     glyph_index_resolver=glyph_resolver))
        drawing = dict(page_count=document['pages'], glyphs=glyphs)
        require(capture.get('table_glyph_projection', 'visible') in ('visible', 'unclipped'),
                'Unknown archived drawing projection')
        require(windows.get('font_source') == 'original-word-pdf', 'Unknown archived font source')
    return archive, capture, document, drawing


def verify(root=ROOT):
    index = read_json(root / INDEX)
    require(index['schema'] == 1 and bool(index['fixtures']), 'Empty platform registry')
    names = set()
    cases = []
    for case in index['fixtures']:
        name = case['mac_case']
        require(name not in names, 'Duplicate shared fixture')
        names.add(name)
        directory = f'tests/word-oracle/mac-elsa-synthetic/{name}'
        source = checked_file(root, case['source'])
        require(case['source']['file'] == f'{directory}/source.docx', 'Wrong Mac source')
        mac_file = checked_file(root, case['mac_manifest'])
        require(case['mac_manifest']['file'] == f'{directory}/manifest.json', 'Wrong Mac manifest')
        mac = read_json(mac_file)
        require(mac['reference']['platform'] == 'mac', 'Mac reference platform changed')
        # Some imported Mac captures predate version recording. Preserve that
        # provenance gap instead of inventing a build or changing their goldens.
        require(mac['reference']['application'] == 'Microsoft Word', 'Wrong Mac application')
        require(mac['sourceSha256'] == case['source']['sha256'], 'Mac source identity changed')
        coordinates = repo_file(root, f'{directory}/pages.json.gz').read_bytes()
        require(hashlib.sha256(coordinates).hexdigest() == mac['coordinatesSha256'],
                'Mac coordinates changed')
        require(len(json.loads(gzip.decompress(coordinates))) == mac['pages'], 'Mac page count')
        require(bool(case['windows']), 'Missing Windows version expectations')
        captures = set()
        for windows in case['windows']:
            expected_context = ('independent-screen-print' if name in INDEPENDENT_OUTPUT_CASES
                                else 'shared-screen-pdf')
            require(windows.get('output_context', 'shared-screen-pdf') == expected_context,
                    'Established screen/PDF coordinate assertions changed')
            require(name not in INDEPENDENT_OUTPUT_CASES or 'archive' in windows,
                    'Independent output contexts require original archived Word outputs')
            if 'archive' in windows:
                require('capture' not in windows and 'drawing_reference' not in windows,
                        'Ambiguous original Windows reference')
                archive, capture, document, fixture = archive_reference(root, case, windows)
                key = (capture['word']['product_version'], capture['capture_id'])
                require(key not in captures, 'Duplicate Word version/capture')
                captures.add(key)
                cases.append((name, source, archive, document, fixture, 'original-word-pdf'))
                continue
            manifest = checked_file(root, windows['capture'])
            capture = read_json(manifest)
            key = (capture['word']['product_version'], capture['capture_id'])
            require(key not in captures, 'Duplicate Word version/capture')
            captures.add(key)
            require(capture['platform'] == 'windows' and capture['status'] == 'complete',
                    'Incomplete or mislabelled Windows capture')
            require(key[0] == windows['word_product_version'], 'Windows Word version changed')
            documents = [d for d in capture['documents'] if d['name'] == windows['fixture']]
            require(len(documents) == 1, 'Missing or duplicate Windows input')
            document = documents[0]
            require(document['sha256'] == case['source']['sha256'], 'Windows source identity changed')
            archived = manifest.parent / 'inputs' / (document['name'] + '.docx')
            require(archived.read_bytes() == source.read_bytes(), 'Windows input differs from Mac original')
            require(hashlib.sha256((manifest.parent / document['pdf']).read_bytes()).hexdigest()
                    == document['pdf_sha256'], 'Original Windows Word PDF changed')
            reference_file = checked_file(root, windows['drawing_reference'])
            require(reference_file.parent == manifest.parent, 'Wrong drawing capture')
            reference = read_json(reference_file)
            require(reference == extract(manifest), 'Drawing origins differ from original Word EMFs')
            require(reference['platform'] == 'windows' and reference['tolerance_pt'] == .375,
                    'Windows platform/tolerance changed')
            fixture = next(f for f in reference['fixtures'] if f['name'] == document['name'])
            require(fixture['docx_sha256'] == case['source']['sha256'], 'Wrong drawing input')
            font_source = windows.get('font_source', 'installed')
            require(font_source in ('installed', 'original-word-pdf'), 'Unknown font source')
            cases.append((name, source, manifest, document, fixture, font_source))
        if name in REQUIRED_CAPTURES:
            require(('16.0.19127.20936', REQUIRED_CAPTURES[name]) in captures,
                    'Previously asserted Word version/capture was removed')
    require(REQUIRED_CAPTURES.keys() <= names, 'Shared fixture coverage removed')
    return cases


def check_windows(executable, output, root=ROOT):
    require(sys.platform == 'win32', 'Windows font measurements require Windows')
    cases = verify(root)
    independent = INDEPENDENT_OUTPUT_CASES
    output.mkdir(parents=True, exist_ok=True)
    checked = 0
    for name, source, manifest, document, drawing, font_source in cases:
        archived = manifest.suffix == '.zip'
        if archived:
            with ZipFile(manifest) as bundle:
                capture = json.loads(bundle.read('capture.json').decode('utf-8-sig'))
                original_pdf_bytes = bundle.read(document['pdf'])
        else:
            capture = read_json(manifest)
        target = output / name / capture['capture_id']
        target.mkdir(parents=True, exist_ok=True)
        actual_path, painted_path = target / 'native.json', target / 'native.pdf'
        if archived:
            original_pdf = target / 'original-word.pdf'
            require(not original_pdf.exists(), 'Refusing to replace extracted original PDF')
            original_pdf.write_bytes(original_pdf_bytes)
        else:
            original_pdf = manifest.parent / document['pdf']
        options = []
        if font_source == 'original-word-pdf':
            extract_fonts(original_pdf, target / 'fonts')
            options = ['--fonts-directory', str(target / 'fonts')]
        subprocess.run([str(executable), str(source), str(actual_path), 'windows',
                        str(painted_path), '--unclipped-tables', *options], check=True)
        actual = read_json(actual_path)
        if name not in independent:
            compare(pdf_origins(original_pdf, True), actual, name + '/PDF-unclipped')
        visible_path = target / 'visible.json'
        subprocess.run([str(executable), str(source), str(visible_path), 'windows', *options], check=True)
        visible = pdf_origins(original_pdf)
        actual_visible = read_json(visible_path)
        for model in (actual, actual_visible):
            require(model['input_sha256'] == hashlib.sha256(source.read_bytes()).hexdigest(),
                    'Wrong shared rendered input')
            if font_source == 'original-word-pdf':
                from check_windows_font_coordinates import require_original_font_programs_registered
                require_original_font_programs_registered(target / 'fonts', model)
        projection = capture.get('table_glyph_projection', 'visible')
        checked += compare(drawing, actual if projection == 'unclipped' else actual_visible,
                           name + '/EMF')
        if name in independent:
            assert_independent_outputs(drawing, actual_visible, actual,
                                       visible_path.with_suffix('.png'), actual_path.with_suffix('.png'),
                                       original_pdf, painted_path, name)
        else:
            compare(visible, actual_visible, name + '/PDF-visible')
        require(painted_path.read_bytes() != original_pdf.read_bytes(), 'Word self-comparison')
        compare(visible, pdf_origins(painted_path), name + '/painted-PDF')
        if name in PHYSICAL_FONT_CASES:
            # Preserve strict XY matching and distinct per-glyph partners
            # while also rejecting physical font face or size regressions.
            for unclipped in (False, True):
                compare(pdf_origins(original_pdf, unclipped, font_metrics=True),
                        pdf_origins(painted_path, unclipped, font_metrics=True),
                        name + '/physical-font-metrics', font_metrics=True)
        if name in PREFERRED_GRID_BORDER_CASES | BORDERLESS_GRID_CASES | MATH_FRACTION_RULE_CASES | BORDERLESS_MATH_CASES | RTL_MERGED_PERCENTAGE_CASES | PARAGRAPH_MARK_MATH_CASES | INHERITED_RIGHT_MATH_CASES | OVERSIZED_MANUAL_BREAK_CASES | ADDITIONAL_AFTER_SPACING_CASES | ROW_AFTER_SPACING_CASES | CONTINUED_HEADER_FRAME_CASES | RTL_SPACED_PERCENTAGE_CASES:
            borders = assert_original_vector_borders(
                original_pdf, painted_path, name, expected_empty=name in BORDERLESS_GRID_CASES | BORDERLESS_MATH_CASES | PARAGRAPH_MARK_MATH_CASES | INHERITED_RIGHT_MATH_CASES | OVERSIZED_MANUAL_BREAK_CASES | ADDITIONAL_AFTER_SPACING_CASES | ROW_AFTER_SPACING_CASES,
                expected_empty_pages=RTL_SPACED_PERCENTAGE_EMPTY_RULE_PAGES.get(name, ()))
            proof = dict(source_sha256=actual['input_sha256'], word=capture['word'],
                         capture_id=capture['capture_id'],
                         original_pdf_sha256=hashlib.sha256(original_pdf.read_bytes()).hexdigest(),
                         native_pdf_sha256=hashlib.sha256(painted_path.read_bytes()).hexdigest(),
                         border_tolerance_pt=.375, pages=borders)
            (target / 'versioned-vector-borders.json').write_text(
                json.dumps(proof, indent=2) + '\n', encoding='utf8')
        print(f'{name}: {document["pages"]} pages, {len(drawing["glyphs"])} glyphs, '
              f'Windows Word {capture["word"]["product_version"]}: PASS', flush=True)
    return checked


def assert_original_vector_borders(word_pdf, painted_pdf, label, *, expected_empty=False,
                                   expected_empty_pages=()):
    from compare_windows_borders import TOLERANCE_PT, compare as compare_borders
    require(TOLERANCE_PT == .375, 'Original vector border tolerance changed')
    results = compare_borders(word_pdf, painted_pdf)
    require(set(expected_empty_pages) <= {p['page'] for p in results},
            label + '/original-vector-borders: expected empty page missing')
    require(bool(results) and all(
        (page['word_rectangles'] == page['native_rectangles'] == 0
         if expected_empty or page['page'] in expected_empty_pages
         else page['word_rectangles'] > 0 and page['native_rectangles'] > 0)
        and page['word_uncovered_count'] == page['native_uncovered_count'] == 0
        for page in results), label + '/original-vector-borders: ' + repr(results))
    return results


def assert_independent_outputs(drawing, screen, snapshot, screen_png, snapshot_png,
                               word_pdf, painted_pdf, label):
    """Keep live origins stable and assert each actual output against Word."""
    compare(drawing, screen, label + '/screen-EMF')
    compare(drawing, snapshot, label + '/export-preserved-EMF')
    require(screen['glyphs'] == snapshot['glyphs']
            and screen['page_count'] == snapshot['page_count'],
            label + ': PDF export changed live logical origins')
    require(screen_png.read_bytes() == snapshot_png.read_bytes(),
            label + ': PDF export changed live screen pixels')
    require(word_pdf.read_bytes() != painted_pdf.read_bytes(), label + ': Word PDF self-comparison')
    for unclipped in (False, True):
        compare(pdf_origins(word_pdf, unclipped), pdf_origins(painted_pdf, unclipped),
                label + ('/vector-PDF-unclipped' if unclipped else '/vector-PDF-visible'))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable', type=Path)
    parser.add_argument('--output-directory', type=Path, default=ROOT / 'target/platform-word-coordinates')
    args = parser.parse_args()
    if args.native_executable:
        print(f'Checked {check_windows(args.native_executable.resolve(), args.output_directory.resolve())} drawing origins')
    else:
        print(f'Verified {len(verify())} shared DOCX/platform-version reference pairs')
