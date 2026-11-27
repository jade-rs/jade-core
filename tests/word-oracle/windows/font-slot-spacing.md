These 434 controls preserve actual Windows Word 16.0.19127.20936 output in
18 capture batches. They cover mixed font sizes and faces, explicit Latin
and East Asian font slots, regular/bold/italic styles, compatibility modes
12/14/15, balanced spacing, manual and natural line endings, and fractional
page margins. There are 422 distinct DOCX byte sequences and 5,705 nonwhite
glyphs. Duplicate source controls intentionally retain separate Word captures.

Each capture retains the original DOCX, PDF, EMF and XPS bytes, the complete
original capture archive, full Word product/file/application versions and
executable SHA-256, original embedded font hashes and font version strings,
capture tools, authoring facts and generation dependency hashes. The portable
generators reproduce every captured DOCX byte. Capture directories and output
artifacts identify the Word version; a later Word build requires new evidence
instead of replacing existing originals.

The required font registry grows from 1,537 to 1,971 cases. All 434 new inputs
must pass EMF, unclipped and visible PDF text, actual painted PDF, both native
and visible XPS coordinate comparisons, and paragraph background geometry.
The font-slot checker consumes the renderer outputs from the preceding font
step. Neither rendering path receives expected Word positions. Negative tests
exercise both axes, missing glyphs, wrong inputs, page/text/order changes and
nonfinite coordinates for every new source. Coverage, original bytes, font
provenance and promotions are also checked; none of these 434 are pending.

The Windows fixes preserve requested script-slot identity while resolving
Japanese paint fonts, search sibling subsets of the same requested face before
foreign fallbacks, and use source font metrics for legacy balanced spacing.
The legacy mixed justification allocator realizes its origin on Word's 600dpi
device grid. This applies inside that allocator, rather than translating all
paragraphs. Font selection caches include the Word layout platform, preventing
Windows selections from being reused after switching a measurer to Mac mode.
Long Windows font paths and repeated identical font-cache links are handled
without changing the font programs.

Mac originals and existing coordinate tolerances are retained. GitHub Actions
continues to require actual macOS native and browser coordinate comparisons,
alongside Windows version-specific checks. Older pending PDF/XPS differences
outside these new controls remain explicitly pending in their own registries;
passing this corpus does not resolve those differences.
