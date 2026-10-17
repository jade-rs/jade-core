"""Our synthetic script-style font; rectangular outlines and fixed timestamps."""
from pathlib import Path
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.otlLib.builder import buildMathTable
from fontTools.feaLib.builder import addOpenTypeFeaturesFromString

order = ['.notdef', 'x', 'x.script', 'x.scriptscript', 's', 's.script',
         'u', 'i', 'i.script', 'integral', 'integral.script', 'integral.scriptscript', 'integral.display']
f = FontBuilder(1000, isTTF=True)
f.setupGlyphOrder(order)
f.setupCharacterMap({**{ord(c): c for c in ['x', 's', 'u', 'i']}, ord('∫'): 'integral'})
glyphs = {}
widths = [500, 500, 650, 800, 500, 700, 450, 500, 600, 500, 600, 700, 800]
for index, name in enumerate(order):
    pen = TTGlyphPen(None)
    if index:
        pen.moveTo((0, 0)); pen.lineTo((widths[index] - 100, 0))
        pen.lineTo((widths[index] - 100, 600 + index * 10)); pen.lineTo((0, 600 + index * 10)); pen.closePath()
    glyphs[name] = pen.glyph()
f.setupGlyf(glyphs)
f.setupHorizontalMetrics({name: (width, 0) for name, width in zip(order, widths)})
f.setupHorizontalHeader(ascent=800, descent=-200)
f.setupNameTable({'familyName': 'Jade Math Script Test', 'styleName': 'Regular',
                  'uniqueFontIdentifier': 'Jade Math Script Test 1.0', 'fullName': 'Jade Math Script Test',
                  'psName': 'JadeMathScriptTest', 'version': 'Version 1.0'})
f.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
f.setupPost(); f.setupMaxp()
buildMathTable(f.font, constants={'ScriptPercentScaleDown': 65, 'ScriptScriptPercentScaleDown': 40, 'DisplayOperatorMinHeight': 1800}, vertGlyphVariants={'integral': [('integral', 1000), ('integral.display', 2200)]})
addOpenTypeFeaturesFromString(f.font, '''
languagesystem DFLT dflt;
languagesystem math dflt;
lookup ScriptAlternates useExtension { sub x from [x.script x.scriptscript]; sub integral from [integral.script integral.scriptscript]; } ScriptAlternates;
lookup ScriptSingle { sub s by s.script; } ScriptSingle;
lookup ScriptIncomplete { sub i from [i.script]; } ScriptIncomplete;
feature ssty { lookup ScriptAlternates; lookup ScriptSingle; lookup ScriptIncomplete; } ssty;
feature salt { sub u by x.script; } salt;
''')
f.font['head'].created = f.font['head'].modified = 2082844800
f.font.recalcTimestamp = False
f.save(Path(__file__).with_name('math-script-font.ttf'))

# Distinct ssty records for Latin and math; put Latin first to catch global lookup.
del f.font['GSUB']
addOpenTypeFeaturesFromString(f.font, '''
languagesystem latn dflt;
languagesystem math dflt;
lookup MathAlternates useExtension { sub x from [x.script x.scriptscript]; } MathAlternates;
feature ssty {
    script latn; sub x by i.script;
    script math; lookup MathAlternates;
} ssty;
''')
table = f.font['GSUB'].table
systems = {r.ScriptTag: r.Script.DefaultLangSys for r in table.ScriptList.ScriptRecord}
latin_index, math_index = systems['latn'].FeatureIndex[0], systems['math'].FeatureIndex[0]
if latin_index > math_index:
    records = table.FeatureList.FeatureRecord
    records[latin_index], records[math_index] = records[math_index], records[latin_index]
    remap = {latin_index: math_index, math_index: latin_index}
    for script in table.ScriptList.ScriptRecord:
        langs = [script.Script.DefaultLangSys] + [r.LangSys for r in script.Script.LangSysRecord]
        for lang in filter(None, langs):
            lang.FeatureIndex = [remap.get(i, i) for i in lang.FeatureIndex]
            lang.ReqFeatureIndex = remap.get(lang.ReqFeatureIndex, lang.ReqFeatureIndex)
f.save(Path(__file__).with_name('math-script-scope-font.ttf'))
