# Windows Word resizing of nested table columns

The 18 DOCX fixtures are outputs saved by Windows Word 16.0.19127.20936
after setting the second nested column to 65 points with `wdAdjustNone`.
The cases include absolute, percentage and automatic table widths, with
fixed and autofit layouts in compatibility modes 12, 14 and 15. Each
manifest record gives the input before the edit and its SHA-256 hash.

The archive contains Word's vector PDF and print-layout drawing for each
saved file. The rendering gate checks both. The native editing test applies
the same resize to each original input and checks the saved layout, grid,
and cell preferences against Word's output, including undo and redo.
