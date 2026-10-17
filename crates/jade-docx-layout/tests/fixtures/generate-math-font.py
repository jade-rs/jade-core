"""Build our small synthetic MATH fixture; no third-party font outlines.

Run with fontTools installed. Font timestamps are fixed for reproducibility.
The display integral has no Unicode mapping and requires glyph-ID painting.
"""
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.otlLib.builder import buildMathTable

ORDER = [".notdef", "x", "integral", "integral.display", "integral.extender"]
font = FontBuilder(1000, isTTF=True)
font.setupGlyphOrder(ORDER)
font.setupCharacterMap({ord("x"): "x", ord("∫"): "integral"})
glyphs = {}
BOUNDS = [None, (0, 0, 400, 700), (0, -100, 200, 900),
          (-40, -400, 320, 1800), (0, 0, 100, 400)]
for name, bounds in zip(ORDER, BOUNDS):
    pen = TTGlyphPen(None)
    if bounds:
        left, bottom, right, top = bounds
        pen.moveTo((left, bottom))
        pen.lineTo((right, bottom))
        pen.lineTo((right, top))
        pen.lineTo((left, top))
        pen.closePath()
    glyphs[name] = pen.glyph()
font.setupGlyf(glyphs)
font.setupHorizontalMetrics({g: (width, bounds[0] if bounds else 0)
                             for g, width, bounds in zip(ORDER, [500, 500, 300, 550, 100], BOUNDS)})
font.setupHorizontalHeader(ascent=800, descent=-400)
font.setupNameTable({"familyName": "Jade Math Test", "styleName": "Regular",
                     "uniqueFontIdentifier": "Jade Math Test 1.0", "fullName": "Jade Math Test",
                     "psName": "JadeMathTest", "version": "Version 1.0"})
font.setupOS2(sTypoAscender=800, sTypoDescender=-400, usWinAscent=1800, usWinDescent=400)
font.setupPost()
font.setupMaxp()
buildMathTable(font.font, constants={
    "ScriptPercentScaleDown": 65, "ScriptScriptPercentScaleDown": 40,
    "DelimitedSubFormulaMinHeight": 1500, "DisplayOperatorMinHeight": 1800,
    "MathLeading": 110, "AxisHeight": 250, "SuperscriptShiftUp": 360,
    "SubscriptShiftDown": 210, "SuperscriptBaselineDropMax": 200,
    "SubscriptBaselineDropMin": 100, "UpperLimitGapMin": 135,
    "UpperLimitBaselineRiseMin": 300, "LowerLimitGapMin": 135,
    "LowerLimitBaselineDropMin": 670, "SpaceAfterScript": 40,
}, italicsCorrections={"integral": 50, "integral.display": 70},
    extendedShapes={"integral.display"}, minConnectorOverlap=20,
    vertGlyphVariants={"integral": [("integral", 1000), ("integral.display", 2200)]},
    vertGlyphAssembly={"integral": ([("integral", 0, 0, 50, 1000),
                                     ("integral.extender", 1, 50, 50, 400),
                                     ("integral.display", 0, 50, 0, 2200)], 70)})
font.font["head"].created = font.font["head"].modified = 2082844800
font.font.recalcTimestamp = False
font.save(Path(__file__).with_name("math-font.ttf"))
