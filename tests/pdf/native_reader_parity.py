"""Check every native corpus page with independent PDF reader engines."""
import argparse
import json
from pathlib import Path

import pypdfium2

from raster_parity import compare, read_rgb, reader_image_reference
from render_pdf import render_pdf


def check_corpus(directory, readers):
    pdfs = sorted(directory.glob("*.pdf"))
    if not pdfs:
        raise ValueError(f"no native corpus PDFs in {directory}")
    failures = []
    pages = 0
    for pdf in pdfs:
        for reader in readers:
            prefix = directory / f"{pdf.stem}-{reader}"
            render_pdf(pdf, 96, prefix, reader)
            with pypdfium2.PdfDocument(pdf) as document:
                digits = len(str(len(document)))
                for index in range(len(document)):
                    source = read_rgb(directory / f"{pdf.stem}-editor-{index + 1}.png")
                    actual_path = Path(f"{prefix}-{index + 1:0{digits}d}.png")
                    actual = read_rgb(actual_path)
                    page = document[index]
                    try:
                        width, height = page.get_size()
                    finally:
                        page.close()
                    expected = reader_image_reference(
                        source, actual.shape[:2], page_width=width / 0.75,
                        page_height=height / 0.75, scale=1, reader=reader)
                    result = compare(expected, actual)
                    pages += 1
                    if result["matches"]:
                        actual_path.unlink()
                    else:
                        failures.append({"pdf": str(pdf), "page": index + 1,
                                         "reader": reader, **result})
        print(f"Checked {pdf.name}", flush=True)
    report = {"documents": len(pdfs), "reader_pages": pages, "failures": failures}
    (directory / "reader-differences.json").write_text(json.dumps(report), encoding="utf-8")
    print(json.dumps(report), flush=True)
    return not failures


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--readers", nargs="+", required=True,
                        choices=["poppler", "pdfium", "coregraphics"])
    args = parser.parse_args()
    raise SystemExit(0 if check_corpus(args.directory, args.readers) else 1)
