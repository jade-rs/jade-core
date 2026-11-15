"""Assert solid paragraph shading against the original Windows Word PDF."""
import math
from zipfile import ZipFile
import xml.etree.ElementTree as ET

import pymupdf

from check_platform_coordinates import require

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def background_color(source):
    with ZipFile(source) as archive:
        document = ET.fromstring(archive.read('word/document.xml'))
    shading = [node for ppr in document.iter(W + 'pPr') for node in ppr.findall(W + 'shd')]
    require(bool(shading) and all(node.get(W + 'val') in (None, 'clear') for node in shading),
            'Expected solid paragraph shading without a pattern')
    colors = {node.get(W + 'fill') for node in shading}
    require(len(colors) == 1, 'Expected one authored paragraph shading color')
    color, = colors
    require(color is not None and len(color) == 6 and color.upper() != 'FFFFFF',
            'Expected nonwhite solid paragraph shading')
    try:
        return tuple(int(color[i:i + 2], 16) / 255 for i in (0, 2, 4))
    except ValueError as error:
        raise ValueError('Invalid shading color') from error


def rectangles(pdf, color):
    result = []
    with pymupdf.open(pdf) as document:
        page_count = len(document)
        for page_number, page in enumerate(document, 1):
            leading_paper = 0
            for drawing in page.get_drawings():
                if drawing.get('fill_opacity', 0) == 0 and drawing.get('stroke_opacity', 0) == 0:
                    continue
                if (drawing['type'] == 'f' and drawing.get('fill') == (1., 1., 1.)
                        and drawing['fill_opacity'] == 1 and drawing['seqno'] == leading_paper):
                    leading_paper += 1
                    continue
                require(drawing['type'] == 'f' and drawing.get('fill') is not None
                        and drawing['fill_opacity'] == 1,
                        'Unexpected shading stroke, translucency or erasure')
                # Word serializes RGB colors with three decimal places.
                require(max(abs(x - y) for x, y in zip(drawing['fill'], color)) <= .001,
                        'Unexpected shading color or erasure')
                require(len(drawing['items']) == 1 and drawing['items'][0][0] == 're',
                        'Nonrectangular paragraph shading')
                bounds = list(drawing['rect'])
                require(all(math.isfinite(value) for value in bounds)
                        and bounds[2] > bounds[0] and bounds[3] > bounds[1],
                        'Invalid shading rectangle')
                result.append([page_number, *bounds])
    require(bool(result), 'Missing paragraph background paint')
    return page_count, sorted(result, key=lambda r: (r[0], r[2], r[1], r[4], r[3]))


def compare_paragraph_backgrounds(source, word_pdf, native_pdf, label):
    color = background_color(source)
    word_pages, expected = rectangles(word_pdf, color)
    native_pages, actual = rectangles(native_pdf, color)
    require(word_pages == native_pages, label + ': background page count')
    require(len(expected) == len(actual), label + ': missing/extra background')
    for word, jade in zip(expected, actual):
        require(word[0] == jade[0], label + ': background page')
        require(all(abs(x - y) <= .375 for x, y in zip(word[1:], jade[1:])),
                label + ': background axis mismatch')
    # A tolerated outer-edge difference must never create a visible internal seam.
    for i, first in enumerate(expected):
        for j, second in enumerate(expected):
            if i != j and first[0] == second[0] and max(
                    abs(first[1] - second[1]), abs(first[3] - second[3]),
                    abs(first[4] - second[2])) <= .01:
                require(abs(actual[i][4] - actual[j][2]) <= .01,
                        label + ': background continuity gap/overlap')
    return len(expected)
