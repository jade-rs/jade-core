# Preserving loaded ZIP members during DOCX export

Loaded documents retain their validated compressed source archive. Unchanged
members are copied with their existing compression method and compressed bytes;
changed and added members use the existing deflate writer. Removed members win
over both. WordprocessingML is still checked and repaired for invalid
`mc:Ignorable` prefixes, including untouched XML. Registered ZIP metadata such as
Info-ZIP Unix UID/GID fields is allowed through the `zip/unreserved` feature.

The editor retains one extra compressed DOCX in memory. Document clones and full
replacement snapshots share it through `Rc<[u8]>`; ordinary undo keeps the
current source. Newly authored media is still compressed during export. This
change does not improve text shaping, pagination, or native key dispatch.

## Reproduce the writer comparison

```sh
cargo run --release --locked -p jade-docx-writer --example profile_media_export -- input.docx
```

The probe alternates the existing map writer and the preserving writer, discards
two warmups per implementation, and reports 13 samples per implementation. Both
outputs are inflated and CRC-checked outside the timed interval; every part must
match the original writer's output (including markup-compatibility repairs).
The synthetic edit is a trailing XML comment in `word/document.xml`.

A deterministic large-member input can be produced without external packages:

```python
import binascii, random, struct, zipfile, zlib
from pathlib import Path
w, h = 2048, 1024
pixels = random.Random(20261026).randbytes(w * h * 4)
scanlines = b"".join(b"\0" + pixels[y*w*4:(y+1)*w*4] for y in range(h))
def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", binascii.crc32(kind + data))
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(scanlines, 6)) + chunk(b"IEND", b"")
with zipfile.ZipFile("media-export.docx", "w", compression=zipfile.ZIP_DEFLATED) as archive:
    archive.writestr("word/document.xml", '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>benchmark</w:t></w:r></w:p></w:body></w:document>')
    archive.writestr("word/media/noise.png", png)
```

This minimal archive benchmarks the writer; it is not a rendered-document or
Word compatibility fixture. Use a complete DOCX to measure application saves.
Absolute times depend on hardware and load. Keep a controlled application test
separate from this component benchmark.

## Regression coverage

- 128 property cases cover additions, overrides, deletions, stored and deflated
  source members; all surviving part contents and CRCs are checked, and untouched
  compressed bytes are compared exactly.
- Registered Unix ZIP extra fields and per-entry comments survive raw copies.
- Untouched and overridden WordprocessingML retain the existing namespace repair.
- Editor export preserves stored source members, and restoring a full document
  replacement snapshot restores the corresponding source archive.
