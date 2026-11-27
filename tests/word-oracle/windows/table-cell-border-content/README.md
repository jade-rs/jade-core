# Windows Word content in cells with collapsed borders

This archive contains 102 vector PDFs and print-layout drawings from
Windows Word 16.0.19127.20936. The fixtures cover ordinary table text,
percentage-width nested tables, and the same nested tables after setting
their second column to 65 points with `wdAdjustNone`. The cases vary the
border widths and horizontal margins in compatibility modes 12, 14 and 15.

The Windows gate checks both layout coordinates and visible PDF glyphs.
For the 39 edited outputs, the manifest also pins the file that Word opened
before the edit. The native editing test uses those pairs for resize,
reopen, undo and redo comparisons.
