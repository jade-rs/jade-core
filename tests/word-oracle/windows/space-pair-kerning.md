These 32 controls retain actual Windows Word 16.0.19127.20936 outputs for
Arial and Times New Roman, compatibility modes 14/15, kerning disabled/enabled,
and the letter pairs AA AA / AA BB / AA VA / AA TA. The source generator
reproduces all DOCX bytes and changes only the documented run properties/text.

Each capture retains original DOCX/PDF/EMF/XPS bytes, complete Word executable
identity, embedded-font versions/program hashes and the capture tools. Every
font input is bound to the same original XPS SHA-256. The complete capture
bundle also retains the original input generator and tool checksums.

CI requires all 32 cases on four original Word font coordinate planes,
paragraph backgrounds and both native/live XPS paths. Negative tests displace
every X/Y origin path and reject wrong inputs, missing text, reordered glyphs,
page changes and nonfinite coordinates. Original Word positions, all earlier
font records, Mac originals and tolerances are unchanged.

These controls distinguish incomplete PDF font inputs from runtime kerning
behavior. Original XPS programs preserve the needed font tables; the renderer
receives source bytes and font programs without reference positions. Font
selection/provenance follows the same strict subset checks as the mixed-script
controls. Native JSON/PDF/PNG and font-version/hash metadata are uploaded with
the Word-version label; standalone decoded font binaries are excluded.
