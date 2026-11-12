# Word for Mac coordinate fixtures

Each fixture contains a DOCX, a compressed coordinate reference extracted from a Word for Mac PDF, and a manifest recording the source, export, and reference digests. The macOS browser coordinate job builds the DOCX renderer from this repository and compares page sizes, visible glyph origins, and image bounds to those references.

To add a fixture, export its DOCX from Word for Mac using **Save As → PDF → Best for printing**. Record the export and SHA-256 digests of the DOCX and PDF in a manifest based on an existing fixture, then run the extractor with PyMuPDF 1.26.5.

```sh
python tests/word-oracle/mac-elsa-synthetic/extract.py path/to/fixture path/to/word-export.pdf
```

Commit the synthetic DOCX, manifest, and generated `pages.json.gz`.

The inline picture rotation fixtures compare body images with and without a
document grid, plus a 270-degree picture in a table cell. Their manifests record
the source bitmap size and the number of complete images per page so the
extractor can exclude the border samples emitted by Quartz.
