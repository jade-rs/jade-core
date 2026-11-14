# Original Windows Word observations

These archives preserve measurements from the installed Windows Word, including
cases Jade does not yet match. They are not accepted Jade expectations. Passing
the archive integrity check does not declare rendering parity. Qualified cases
remain asserted separately by the platform, font and existing version registries.

Each version directory contains immutable capture ZIPs. Each ZIP retains the
original `capture.json`, encrypted or plain original input DOCX bytes, original
Word-exported PDF, and raw `Page.EnhMetaFileBits` for every physical page. The
capture metadata records Word product/file/application versions, executable hash,
OS, UTC timestamp, input/PDF/EMF hashes and page geometry. Original metadata bytes
are preserved even when their source paths describe this workstation.

`index.json` maps every opened filename and input hash to repository sources,
including imported Mac originals. Renaming a DOCX can alter a Word FILENAME
field; use the recorded opened filename when interpreting that difference.
Never replace Mac expectations with these Windows measurements. Add evidence
for a new Word version in another version directory rather than overwriting it.

CI verifies archive hashes, original metadata, every PDF/EMF/input hash, exact
repository source identity, complete physical pages and established coverage.
The catalog also records an unchanged Mac original that this Windows Word
rejects; no Windows coordinates are fabricated for that input.
