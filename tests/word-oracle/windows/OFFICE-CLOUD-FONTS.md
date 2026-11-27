# Windows Office font discovery

`office-cloud-font-observations.json` retains eight actual Word captures for
Aptos/Aptos Display at 12/20pt and 120/240pt text widths, including two-page
documents. It records the Word build, executable hash, original output hashes,
embedded font versions/hashes, and full font program identities observed in
this PC's Office cache. Full private font files are not copied into the corpus.

The previous native default lookup missed these Office fonts. All eight
documents differed from Word without a directory override; automatic discovery
now matches their original EMF, PDF text and independently painted PDF origins.
Those historical baseline flags are observations, not replacement expectations.

The Windows CI gate constructs an isolated `LOCALAPPDATA` cache from each
original Word PDF's embedded programs. It uses a generation name other than
`4`, removes `JADE_OFFICE_FONT_DIR` from the child process, and runs the normal
native system-font path. It checks that the regular body face came from that
cache with the original program hash, then asserts all four coordinate planes.
The previous executable fails this gate for missing cached body fonts.
For each source the gate also renders Mac mode with empty and populated
Windows cache profiles, requiring identical glyphs, pages, screen pixels and
painted PDF geometry on every page. Actual Mac Word references remain separate.

`JADE_OFFICE_FONT_DIR` retains its explicit priority. Automatic cloud discovery
applies only to Windows layout on Windows. Windows and Mac layout use separate
system-library caches there, preserving the Mac inventory even when Windows
initializes its cache first. macOS keeps its existing font discovery.
