"""Extract the Word print coordinates. Requires PyMuPDF 1.26.5."""
import argparse
import gzip
import json
import unicodedata
from hashlib import sha256
from itertools import chain, groupby
from pathlib import Path
from xml.etree import ElementTree

import fitz


def glyphs(page):
    flags = fitz.TEXTFLAGS_RAWDICT & ~fitz.TEXT_PRESERVE_IMAGES | fitz.TEXT_INHIBIT_SPACES
    for block in page.get_text("rawdict", flags=flags)["blocks"]:
        for line in block["lines"]:
            for span in line["spans"]:
                if span["font"] == "AppleColorEmoji":
                    continue
                for char in span["chars"]:
                    if not char["c"].isspace():
                        yield [char["c"], *[round(value, 3) for value in char["origin"]]]


def painted_glyphs(page):
    for span in page.get_texttrace():
        if span["font"] == "AppleColorEmoji" or span["type"] == 3:
            continue
        for codepoint, _, origin, _ in span["chars"]:
            char = chr(codepoint)
            if not char.isspace():
                yield [char, *[round(value, 3) for value in origin]]


def color_emoji(page, images):
    spans = [span for span in page.get_texttrace() if span["font"] == "AppleColorEmoji"]
    if not spans:
        return
    paint_log = page.get_bboxlog()
    for span in spans:
        # Word draws the color glyph as a bitmap and overlays invisible text.
        # The trace retains the shared origin of code points in a single glyph.
        sequence = span["seqno"]
        if sequence == 0 or span["opacity"] != 0 or tuple(span["dir"]) != (1, 0):
            raise ValueError(f"Unexpected emoji text on page {page.number + 1}")
        operation, bounds = paint_log[sequence - 1]
        x, y = span["chars"][0][2]
        if operation != "fill-image" or abs(bounds[0] - x) > 1 or abs(bounds[3] - y) > 1:
            raise ValueError(f"Missing emoji bitmap on page {page.number + 1}")
        images.remove(bounds)
        for origin, chars in groupby(span["chars"], key=lambda char: char[2]):
            yield ["".join(chr(char[0]) for char in chars), *[round(value, 3) for value in origin]]


def rectangular_clip_image_bounds(page):
    mupdf = fitz.mupdf
    buffer = mupdf.FzBuffer(1024)
    output = mupdf.FzOutput(buffer)
    device = mupdf.fz_new_trace_device(output)
    mupdf.fz_run_display_list(
        page.get_displaylist().this, device, mupdf.FzMatrix(),
        mupdf.FzRect(mupdf.FzRect.Fixed_INFINITE), mupdf.FzCookie(),
    )
    device.fz_close_device()
    output.fz_close_output()
    trace = bytes(buffer.fz_buffer_storage_memoryview())
    clips = [page.rect]
    images = []
    for operation in ElementTree.fromstring(b"<trace>" + trace + b"</trace>"):
        if operation.tag == "clip_path":
            commands = [command.tag for command in operation]
            if commands != ["moveto", "lineto", "lineto", "lineto", "closepath"]:
                raise ValueError(f"Unsupported PDF clipping path on page {page.number + 1}")
            matrix = fitz.Matrix(*map(float, operation.attrib["transform"].split()))
            corners = [
                tuple(fitz.Point(float(command.attrib["x"]), float(command.attrib["y"])) * matrix)
                for command in list(operation)[:-1]
            ]
            x, y = zip(*corners)
            rect = fitz.Rect(min(x), min(y), max(x), max(y))
            edges = zip(corners, corners[1:] + corners[:1])
            if (rect.is_empty
                or set(corners) != {tuple(rect.tl), tuple(rect.tr), tuple(rect.br), tuple(rect.bl)}
                or any(x1 != x2 and y1 != y2 for (x1, y1), (x2, y2) in edges)):
                raise ValueError(f"Nonrectangular PDF clipping path on page {page.number + 1}")
            clips.append(clips[-1] & rect)
        elif operation.tag == "pop_clip":
            clips.pop()
        elif operation.tag == "fill_image":
            matrix = fitz.Matrix(*map(float, operation.attrib["transform"].split()))
            if matrix.b != 0 or matrix.c != 0:
                raise ValueError(f"Rotated PDF image on page {page.number + 1}")
            visible = (fitz.Rect(0, 0, 1, 1) * matrix) & clips[-1]
            if not visible.is_empty:
                images.append(tuple(visible))
        elif operation.tag.startswith("clip_") or operation.tag in {"begin_mask", "begin_tile", "fill_image_mask"}:
            raise ValueError(f"Unsupported PDF image or clip operation on page {page.number + 1}")
    if len(clips) != 1:
        raise ValueError(f"Unbalanced PDF clips on page {page.number + 1}")
    return images


def page_geometry(
    page, *, body_only, print_registration=False, rectangular_image_clips=False,
    source_image_size=None, source_image_count=None, capture_painted_glyphs=False,
):
    header, body, footer = [], [], []
    image_info = page.get_image_info()
    images = [image["bbox"] for image in image_info]
    image_count = len(images)
    # These cutoffs lie in the blank gaps between the stories in this fixture.
    text_glyphs = painted_glyphs(page) if capture_painted_glyphs else glyphs(page)
    for glyph in chain(text_glyphs, color_emoji(page, images)):
        story = body
        if not body_only:
            story = header if glyph[2] < 60 else footer if glyph[2] > 786 else body
        story.append(glyph)
    if rectangular_image_clips:
        if len(images) != image_count:
            raise ValueError(f"Unexpected emoji bitmap in clipped image fixture on page {page.number + 1}")
        images = rectangular_clip_image_bounds(page)
    if source_image_size is not None:
        # Quartz emits small border samples around rotated source images.
        # Their bounds describe sampling operations rather than document objects.
        if rectangular_image_clips or len(images) != image_count:
            raise ValueError(f"Unexpected image extraction mode on page {page.number + 1}")
        width, height = source_image_size
        images = []
        for image in image_info:
            size = (image["width"], image["height"])
            if size == (width, height):
                images.append(image["bbox"])
            elif size not in {(1, 1), (1, height), (width, 1)}:
                raise ValueError(f"Unexpected image size {size} on page {page.number + 1}")
        if len(images) != source_image_count:
            raise ValueError(f"Expected {source_image_count} source images on page {page.number + 1}")
    geometry = {
        "size": [round(page.rect.width, 3), round(page.rect.height, 3)],
        "header": header,
        "body": body,
        "footer": footer,
        "images": [[round(value, 3) for value in bounds] for bounds in images],
    }
    if print_registration:
        # The Aptos probes have one separately positioned Yu Mincho space
        # following each ASCII run. Its text matrix gives the true line end
        # independently of any rounding within the ASCII run's PDF operators.
        line_ends = [
            [round(value, 3) for value in span["chars"][0][2]]
            for span in page.get_texttrace()
            if span["font"] == "YuMincho-Regular"
            and len(span["chars"]) == 1
            and span["chars"][0][0] == ord(" ")
        ]
        if not body_only or [y for _, y in line_ends] != sorted({glyph[2] for glyph in body}):
            raise ValueError(f"Unexpected print registration markers on page {page.number + 1}")
        geometry["bodyLineEnds"] = line_ends
    return geometry


def arabic_spans(body, expected_runs):
    glyphs = [
        (text, x, y)
        for text, x, y in body
        if any("ARABIC" in unicodedata.name(char, "") for char in text)
    ]
    spans = []
    remaining = iter(glyphs)
    for run in expected_runs:
        expected = run if isinstance(run, str) else run[0]
        text = ""
        origins = []
        baselines = set()
        while len(text) < len(expected):
            glyph = next(remaining, None)
            if glyph is None:
                raise ValueError(f"Missing Arabic glyph for {expected!r}")
            char, x, y = glyph
            text = unicodedata.normalize("NFKC", text + char)
            origins.append(x)
            if not unicodedata.category(char[0]).startswith("M"):
                baselines.add(y)
        if text != expected or len(baselines) != 1:
            raise ValueError(f"Word Arabic run differs from the expected text or crosses baselines: {expected!r}")
        spans.append([text, min(origins), baselines.pop()])
    if next(remaining, None) is not None:
        raise ValueError("Unassigned Arabic glyphs in the Word export")
    return spans


def shaped_glyph_clusters(body, expected_clusters):
    """Group Word's visual-order glyphs for a logical DOCX cluster."""
    output = []
    start = 0
    for visual, logical in expected_clusters:
        thai = bool(logical) and all("\u0e00" <= char <= "\u0e7f" for char in logical)
        visual_text = "".join(visual)
        logical_text = logical
        if thai:
            # Word's PDF splits sara am and can encode a raised mai tho as
            # an apostrophe above another mark. Keep the original base origin.
            logical_text = logical.replace("\u0e33", "\u0e4d\u0e32")
            if logical.endswith("\u0e49") and len(logical) > 2 and visual_text.endswith("'"):
                visual_text = visual_text[:-1] + "\u0e49"
        if not visual or sorted(visual_text) != sorted(logical_text):
            raise ValueError(f"Invalid shaped glyph cluster: {visual!r}, {logical!r}")
        for index in range(start, len(body) - len(visual) + 1):
            glyphs = body[index:index + len(visual)]
            if [glyph[0] for glyph in glyphs] != visual:
                continue
            baseline = glyphs[0][2]
            baseline_tolerance = 0.65 if thai else 0
            if any(abs(glyph[2] - baseline) > baseline_tolerance for glyph in glyphs):
                raise ValueError(f"Shaped glyph cluster crosses baselines: {visual!r}")
            output.extend(body[start:index])
            x = glyphs[0][1] if thai else min(glyph[1] for glyph in glyphs)
            output.append([logical, x, baseline])
            start = index + len(visual)
            break
        else:
            raise ValueError(f"Missing shaped glyph cluster in the Word export: {visual!r}")
    output.extend(body[start:])
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fixture", type=Path, help="directory containing source.docx and manifest.json")
    parser.add_argument("pdf", type=Path, help="reference PDF captured in Word")
    args = parser.parse_args()
    fixture, pdf = args.fixture, args.pdf
    manifest_path = fixture / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    if sha256(pdf.read_bytes()).hexdigest() != manifest["reference"]["pdfSha256"]:
        raise ValueError("PDF hash differs from the captured Word reference")
    if sha256((fixture / "source.docx").read_bytes()).hexdigest() != manifest["sourceSha256"]:
        raise ValueError("DOCX hash differs from the captured source")
    with fitz.open(pdf) as document:
        if manifest["reference"]["export"] in {"Save As > PDF", "Save As > PDF > Best for printing"}:
            is_word_pdf = document.metadata["producer"].endswith("Quartz PDFContext")
        else:
            is_word_pdf = document.metadata["creator"] == "Word"
        if not is_word_pdf or len(document) != manifest["pages"]:
            raise ValueError("Unexpected Word PDF metadata or page count")
        source_images = manifest.get("pdfSourceImages")
        source_image_counts = source_images["counts"] if source_images else [None] * len(document)
        if len(source_image_counts) != len(document):
            raise ValueError("Source image counts have the wrong page count")
        pages = [
            page_geometry(
                page,
                body_only=manifest.get("bodyOnly", False),
                print_registration="horizontalPrintRegistration" in manifest,
                rectangular_image_clips=manifest.get("rectangularImageClips", False),
                source_image_size=source_images["sizePx"] if source_images else None,
                source_image_count=source_image_count,
                capture_painted_glyphs=manifest.get("capturePaintedGlyphs", False),
            )
            for page, source_image_count in zip(document, source_image_counts)
        ]
    if "arabicRuns" in manifest:
        runs = manifest["arabicRuns"]
        if len(runs) != len(pages):
            raise ValueError("Arabic run recipe has the wrong page count")
        for page, expected_runs in zip(pages, runs):
            page["arabicSpans"] = arabic_spans(page["body"], expected_runs)
    if "shapedGlyphClusters" in manifest:
        clusters = manifest["shapedGlyphClusters"]
        if len(clusters) != len(pages):
            raise ValueError("Shaped glyph cluster recipe has the wrong page count")
        for page, expected_clusters in zip(pages, clusters):
            page["body"] = shaped_glyph_clusters(page["body"], expected_clusters)
    data = gzip.compress(json.dumps(pages, ensure_ascii=False, separators=(",", ":")).encode(), mtime=0)
    (fixture / "pages.json.gz").write_bytes(data)
    manifest["coordinatesSha256"] = sha256(data).hexdigest()
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
