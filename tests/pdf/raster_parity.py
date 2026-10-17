"""Compare an editor raster with a PDF reader raster, independent of DOCX layout."""
import numpy as np
from PIL import Image


def read_rgb(path):
    image = Image.open(path).convert("RGBA")
    white = Image.new("RGBA", image.size, "white")
    return np.asarray(Image.alpha_composite(white, image).convert("RGB"))


def geometry(reference, actual):
    height = max(reference.shape[0], actual.shape[0])
    width = max(reference.shape[1], actual.shape[1])
    source = np.full((height, width, 3), 255, dtype=np.uint8)
    target = source.copy()
    source[:reference.shape[0], :reference.shape[1]] = reference
    target[:actual.shape[0], :actual.shape[1]] = actual
    source_ink = source.min(axis=2) < 192
    target_ink = target.min(axis=2) < 192

    def neighbors(mask):
        padded = np.pad(mask, 1)
        return np.logical_or.reduce([
            padded[y:y + height, x:x + width]
            for y in range(3) for x in range(3)
        ])

    misplaced = np.count_nonzero(source_ink & ~neighbors(target.min(axis=2) < 224))
    misplaced += np.count_nonzero(target_ink & ~neighbors(source.min(axis=2) < 224))
    ink = np.count_nonzero(source_ink | target_ink)
    return {"matches": bool(misplaced <= ink // 500 + 2), "ink": int(ink),
            "misplaced": int(misplaced)}


def appearance(reference, actual):
    """Keep reader edge sampling flexible while pinning color and local ink mass."""
    height = max(reference.shape[0], actual.shape[0])
    width = max(reference.shape[1], actual.shape[1])
    source = np.full((height, width, 3), 255, dtype=np.int16)
    target = source.copy()
    source[:reference.shape[0], :reference.shape[1]] = reference
    target[:actual.shape[0], :actual.shape[1]] = actual

    different = (source != target).any(axis=2)
    rows = np.flatnonzero(different.any(axis=1))
    columns = np.flatnonzero(different.any(axis=0))
    if not len(rows):
        return {"color_pixels": 0, "coverage_tiles": 0}
    # Identical margins need no color/coverage work. Keep whole original
    # 16-pixel windows and a surrounding window for every changed pixel.
    top = max(0, int(rows[0]) // 16 * 16 - 16)
    left = max(0, int(columns[0]) // 16 * 16 - 16)
    bottom = min(height, (int(rows[-1]) // 16 + 2) * 16)
    right = min(width, (int(columns[-1]) // 16 + 2) * 16)
    source = source[top:bottom, left:right]
    target = target[top:bottom, left:right]
    height, width = source.shape[:2]

    def color_failures(first, second):
        # Equal center colors already match. Inspect only the remaining pixels,
        # so a reader's small paper-color rounding cannot force nine full-page
        # neighborhood scans at high export density.
        y, x = np.nonzero(np.abs(first - second).max(axis=2) > 8)
        if not len(y):
            return 0
        center = first[y, x]
        padded = np.pad(first, ((2, 2), (2, 2), (0, 0)), constant_values=255)
        minimum = center.copy()
        maximum = center.copy()
        for dy in range(5):
            for dx in range(5):
                neighbor = padded[y + dy, x + dx]
                np.minimum(minimum, neighbor, out=minimum)
                np.maximum(maximum, neighbor, out=maximum)
        stable = (maximum - minimum).max(axis=1) <= 2
        other = np.pad(second, ((1, 1), (1, 1), (0, 0)), constant_values=255)
        matched = np.zeros(len(y), dtype=bool)
        for dy in range(3):
            for dx in range(3):
                matched |= np.abs(center - other[y + dy, x + dx]).max(axis=1) <= 8
        return int(np.count_nonzero(stable & ~matched))

    def masses(rgb):
        ink = np.pad(255 - rgb, ((1, 1), (1, 1), (0, 0)))
        integral = np.pad(ink.cumsum(axis=0).cumsum(axis=1), ((1, 0), (1, 0), (0, 0)))
        grids = []
        # Overlapping windows prevent a rule crossing a window boundary from
        # hiding a doubled width within the neighboring-sampling envelope.
        for phase_y, phase_x in [(0, 0), (0, 8), (8, 0), (8, 8)]:
            y = np.arange(phase_y, height, 16)[:, None]
            x = np.arange(phase_x, width, 16)[None, :]
            bottom = np.minimum(y + 16, height)
            right = np.minimum(x + 16, width)
            grids.append([integral[bottom + dy, right + dx] - integral[y + dy, right + dx]
                          - integral[bottom + dy, x + dx] + integral[y + dy, x + dx]
                          for dy in range(3) for dx in range(3)])
        return grids

    source_mass = masses(source)
    target_mass = masses(target)

    def mass_failures(first, second):
        center = first[4]
        minimum = np.minimum.reduce(second)
        maximum = np.maximum.reduce(second)
        # Fractional image sampling interpolates neighboring windows, rather
        # than choosing an integer offset. Pin mass to that local envelope.
        # The allowance never grows with unrelated ink elsewhere on the page.
        lower = minimum - np.maximum(center, minimum) * 0.1 - 127.5
        upper = maximum + np.maximum(center, maximum) * 0.1 + 127.5
        matched = ((center >= lower) & (center <= upper)).all(axis=2)
        return int(np.count_nonzero(~matched))

    colors = color_failures(source, target) + color_failures(target, source)
    coverage = sum(mass_failures(first, second) + mass_failures(second, first)
                   for first, second in zip(source_mass, target_mass))
    return {"color_pixels": colors, "coverage_tiles": coverage}



def compare(reference, actual):
    result = geometry(reference, actual)
    result.update(appearance(reference, actual))
    result["page_size_matches"] = all(
        0 <= observed - expected <= 1
        for expected, observed in zip(reference.shape[:2], actual.shape[:2])
    )
    result["matches"] &= result["color_pixels"] == 0 and result["coverage_tiles"] == 0
    result["matches"] &= result["page_size_matches"]
    return result




def reader_image_reference(reference, shape, *, page_width, page_height, scale, reader,
                           captured_density=True):
    """Predict a reader's integer viewport sampling without registering images."""
    import math
    viewport_height, viewport_width = shape
    if any(abs(observed - math.ceil(logical * scale)) > 1
           for observed, logical in [(viewport_height, page_height), (viewport_width, page_width)]):
        return reference
    if reader not in ("pdfium", "poppler"):
        return reference
    height, width = reference.shape[:2]
    # These readers rasterize into an integer viewport and round image bounds.
    # Their image sampler interpolates from pixel edges, rather than centers.
    def image_extent(pixels, viewport, logical):
        if reader == "poppler":
            # Splash uses the requested DPI's continuous transform. Its upper
            # image bound is inclusive, independently of rounded page pixels.
            extent = pixels if captured_density else logical * scale
            return math.floor(extent + 1e-7) + 1
        extent = pixels * viewport / (logical * scale) if captured_density else viewport
        # Splash includes both integer endpoints of the transformed image box;
        # PDFium uses an exclusive endpoint. This is observable even at 1:1
        # density, where Poppler interpolates N pixels into N+1 and clips.
        return math.ceil(extent - 1e-7)

    image_width = image_extent(width, viewport_width, page_width)
    image_height = image_extent(height, viewport_height, page_height)
    origin_x = origin_y = 0
    if reader == "pdfium":
        # PDFium composes its page and image matrices in float precision.
        # A top-aligned image can start slightly below zero before its outer
        # rectangle is rounded, changing both origin and sample dimensions.
        page_points = page_height * 0.75
        image_points = height / scale * 0.75 if captured_density else page_points
        factor = np.float32(viewport_height / np.float32(page_points))
        device_height = np.float32(np.float32(image_points) * factor)
        bottom = np.float32(np.float32(viewport_height) -
                            np.float32(np.float32(page_points - image_points) * factor))
        top_origin = np.float32(bottom - device_height)
        origin_y = math.floor(float(top_origin))
        image_height = math.ceil(float(np.float32(device_height + top_origin))) - origin_y
        x_factor = np.float32(viewport_width / np.float32(page_width * 0.75))
        image_width = math.ceil(float(np.float32(
            np.float32(width / scale * 0.75 if captured_density else page_width * 0.75) * x_factor)))
    if reader == "poppler":
        def pdf_decimal(value):
            # PDF stores decimal coordinates. Poppler accumulates fractional
            # digits, which can differ from Python's correctly rounded float
            # conversion at an inclusive integer image boundary.
            from decimal import Decimal
            literal = format(Decimal(str(value)), "f")
            integer, _, fraction = literal.lstrip("-").partition(".")
            result = float(integer or "0")
            place = 0.1
            for digit in fraction:
                result += int(digit) * place
                place *= 0.1
            return -result if value < 0 else result

        # Reproduce the PDF CTM arithmetic before applying Splash's floor.
        # Cancelling top-alignment translations algebraically loses the tiny
        # negative rounding which can move its inclusive image box by a row.
        dpi_factor = 96 * scale / 72
        page_points = page_height * 0.75
        image_points = height / scale * 0.75 if captured_density else page_points
        bottom = pdf_decimal(page_points) * dpi_factor - pdf_decimal(page_points - image_points) * dpi_factor
        device_height = pdf_decimal(image_points) * dpi_factor
        top_origin = bottom - device_height
        origin_y = math.floor(top_origin)
        image_height = math.floor(device_height + top_origin) + 1 - origin_y
        image_width = math.floor(pdf_decimal(width / scale * 0.75 if captured_density else page_width * 0.75) * dpi_factor) + 1
    top, left = max(0, origin_y), max(0, origin_x)
    bottom = min(viewport_height, origin_y + image_height)
    right = min(viewport_width, origin_x + image_width)
    dtype = np.float64 if reader == "poppler" else np.float32
    xs = (np.arange(left, right, dtype=dtype) - origin_x) * width / image_width
    ys = (np.arange(top, bottom, dtype=dtype) - origin_y) * height / image_height
    x0 = np.minimum(xs.astype(int), width - 1)
    wx = (xs - x0)[None, :, None]
    result = np.full((viewport_height, viewport_width, 3), 255, dtype=np.uint8)
    # Bound floating-point intermediates even for a full 4x A4 page.
    # Each output chunk needs only its source rows and their next neighbors.
    for start in range(0, len(ys), 64):
        positions = ys[start:start + 64]
        y0 = np.minimum(positions.astype(int), height - 1)
        y1 = np.minimum(y0 + 1, height - 1)
        source_rows, indices = np.unique(np.concatenate((y0, y1)), return_inverse=True)
        row = reference[source_rows[:, None], x0].astype(dtype) * (1 - wx)
        row += reference[source_rows[:, None], np.minimum(x0 + 1, width - 1)].astype(dtype) * wx
        if reader == "poppler":
            # Splash quantizes expandRow to bytes before interpolating rows.
            row = row.astype(np.uint8)
        wy = (positions - y0)[:, None, None]
        sampled = row[indices[:len(y0)]] * (1 - wy) + row[indices[len(y0):]] * wy
        result[top + start:top + start + len(y0), left:right] = (
            sampled if reader == "poppler" else np.rint(sampled)).astype(np.uint8)
    return result


if __name__ == "__main__":
    import argparse
    import json
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", help="JSON array of reference/actual PNG paths")
    args = parser.parse_args()
    with open(args.manifest, encoding="utf-8") as stream:
        pairs = json.load(stream)
    failures = []
    for pair in pairs:
        reference = read_rgb(pair["reference"])
        actual = read_rgb(pair["actual"])
        if "page_width" in pair:
            reference = reader_image_reference(
                reference, actual.shape[:2], page_width=pair["page_width"],
                page_height=pair["page_height"], scale=pair["scale"], reader=pair["reader"],
            )
        result = compare(reference, actual)
        if not result["matches"]:
            failures.append({**pair, **result})
    print(json.dumps({"pages": len(pairs), "failures": failures}))
    raise SystemExit(1 if failures else 0)
