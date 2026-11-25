# Windows Word manual page breaks

These probes use Microsoft Word 16.0.19127.20936 on Windows. The DOCX
compatibility modes 12, 14 and 15 are settings in the inputs. The pinned archive
contains the original read-only Word PDF exports, executable identity, input
and output hashes, and the DOCX files saved after Word's `Selection.InsertBreak(7)`
operation. That operation was exercised at the start, middle and end of a
paragraph, over a selection, in a list and inside a table.

The page-break controls distinguish these cases.

| Input | Word placement |
| --- | --- |
| Text before and after the break in one paragraph | Text continues on the next page with continuation indentation. Putting the break in its own run produces the same placement. |
| Break at the start of a paragraph | The text keeps its first-line indentation. A list label moves to the same page as the text. |
| Leading break at the document or section start | The first text page retains paragraph before spacing. |
| Leading break whose empty line box cannot fit | Word advances once and retains before spacing on the next page. |
| Paragraph-final or standalone break | The following paragraph starts on the next page. |
| `pageBreakBefore` on a paragraph in ordinary flow | Modes 12 and 14 collapse its before spacing against the previous paragraph's after spacing. Mode 15 omits its before spacing. |
| `lastRenderedPageBreak` | A saved layout marker leaves text on its current page. |

In the saved Word insertion samples, a break in the middle of body text splits
the paragraph and inserts a dedicated break paragraph between the halves.
At the start, Word inserts the break paragraph before the existing paragraph.
After an end-of-paragraph insertion, the next page has an empty paragraph for
the caret. From a table cell, Word inserts the break paragraph before the
body table.

`scripts/word-oracle/generate_windows_page_break_cases.py` regenerates the
DOCX inputs byte for byte. `check_windows_manual_page_breaks.py` compares page
counts and absolute glyph origins for 102 inputs in both the native layout
snapshot and the exported vector PDF. It also checks the PDF text order.
The tolerance is 0.375 pt on each axis.
CI extracts the fonts from Word's original PDFs. The native round-trip test
checks the paragraph boundaries and caret against the saved Word DOCX
samples, then exports, reopens, undoes and redoes the edit.
