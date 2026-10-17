"""Rasterize with independent PDF engines; keep reader defaults unchanged."""
import argparse
import os
from pathlib import Path
import subprocess


def render_pdf(pdf, dpi, prefix, reader):
    if not dpi > 0:
        raise ValueError("density must be positive")
    Path(prefix).parent.mkdir(parents=True, exist_ok=True)
    if reader == "poppler":
        subprocess.run([os.environ.get("JADE_PDFTOPPM", "pdftoppm"),
                        "-png", "-r", str(dpi), str(pdf), str(prefix)], check=True)
    elif reader == "coregraphics":
        subprocess.run([os.environ["JADE_PDF_COREGRAPHICS"],
                        str(pdf), str(dpi), str(prefix)], check=True)
    elif reader == "pdfium":
        import pypdfium2
        with pypdfium2.PdfDocument(pdf) as document:
            digits = len(str(len(document)))
            for index in range(len(document)):
                page = document[index]
                try:
                    bitmap = page.render(scale=dpi / 72.0)
                    try:
                        bitmap.to_pil().save(f"{prefix}-{index + 1:0{digits}d}.png")
                    finally:
                        bitmap.close()
                finally:
                    page.close()
    else:
        raise ValueError(f"unknown PDF reader: {reader}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reader", choices=["poppler", "pdfium", "coregraphics"])
    parser.add_argument("pdf", type=Path)
    parser.add_argument("dpi", type=float)
    parser.add_argument("prefix", type=Path)
    args = parser.parse_args()
    render_pdf(args.pdf, args.dpi, args.prefix, args.reader)
