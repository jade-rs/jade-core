# Windows Word preferred width at the nested-table resize boundary

The 12 fixtures were saved by Windows Word 16.0.19127.20936 after resizing
the second column of a percentage-width nested table. The requested width
is in the capture manifest. Compatibility modes 12, 14 and 15 each cover
one twip below the enclosing frame, the exact frame, one twip above it,
and a wider overflow. Word saves the overflowing grids in points.

The archive pins the inputs before and after each resize and includes the
saved documents' Word PDFs and print-layout drawings. The native test replays
the operation and checks the model, saved file, undo and redo against Word.
