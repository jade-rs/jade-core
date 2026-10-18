"""Font-free regressions with literal, independently calculated sRGB8 goldens.

The goldens use IEC sRGB transfer equations and Python colorsys HLS, with
rounding only at the output. This generator never calls the application.
"""
import json
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
from generate_colors import ENTRIES


def rgb(value, children=""):
    return f'<a:srgbClr val="{value}">{children}</a:srgbClr>'


def ops(**values):
    return "".join(f'<a:{name} val="{value}"/>' for name, value in values.items())


def scrgb(value, children=""):
    r, g, b = value
    return f'<a:scrgbClr r="{r}" g="{g}" b="{b}">{children}</a:scrgbClr>'


def hsl(hue, sat, lum, children=""):
    return f'<a:hslClr hue="{hue}" sat="{sat}" lum="{lum}">{children}</a:hslClr>'


def scheme(slot, children):
    return f'<a:schemeClr val="{slot}">{children}</a:schemeClr>'


CASES = [
    ("channels-absolute", [rgb("123456", ops(red=50000, green=25000, blue=75000)), rgb("123456", ops(red=-10000, green=125000, blue=0))], [[188, 137, 225], [0, 255, 0]]),
    ("channels-modulated", [rgb("123456", ops(redMod=50000, greenMod=125000, blueMod=200000)), rgb("FFFFFF", ops(redMod=-50000, greenMod=200000, blueMod=25000))], [[10, 58, 119], [0, 255, 137]]),
    ("channels-offset", [rgb("123456", ops(redOff=20000, greenOff=-10000, blueOff=50000)), rgb("123456", ops(redOff=-10000, greenOff=200000, blueOff=-200000))], [[125, 0, 202], [0, 255, 0]]),
    ("channels-ordered", [rgb("123456", ops(redMod=200000, redOff=-20000, greenMod=50000, greenOff=20000, blueMod=200000, blueOff=50000)), rgb("123456", ops(redOff=-20000, redMod=200000, greenOff=20000, greenMod=50000, blueOff=50000, blueMod=200000))], [[0, 128, 216], [0, 96, 255]]),
    ("hsl-absolute", [rgb("123456", ops(hue=7200000, sat=50000, lum=60000)), rgb("FF0000", ops(hue=10800000, sat=75000, lum=30000))], [[102, 204, 102], [19, 134, 134]]),
    ("hsl-modulation", [rgb("123456", ops(hueMod=50000, satMod=50000, lumMod=125000)), rgb("123456", ops(satMod=-50000, lumMod=50000))], [[54, 86, 44], [26, 26, 26]]),
    ("hsl-offset", [rgb("123456", ops(hueOff=-7200000, satOff=25000, lumOff=20000)), rgb("123456", ops(hueOff=25200000, satOff=-125000, lumOff=-25000))], [[103, 196, 10], [0, 0, 0]]),
    ("hsl-chain", [rgb("333333", ops(shade=30000, lumMod=50000, lumOff=10000) + ops(shade=30000)), rgb("123456", ops(shade=70000, hueOff=25200000, satMod=30000, lumMod=125000) + ops(shade=70000))], [[18, 18, 18], [44, 35, 54]]),
    ("scrgb", [scrgb([50000, 50000, 50000]), scrgb([25000, 50000, 75000])], [[188, 188, 188], [137, 188, 225]]),
    ("hsl", [hsl(0, 100000, 25000), hsl(10800000, 75000, 30000)], [[128, 0, 0], [19, 134, 134]]),
    ("system", ['<a:sysClr val="window" lastClr="FF0000"/>', '<a:sysClr val="window" lastClr="00FF00"/>'], [[255, 0, 0], [0, 255, 0]]),
    ("alpha-precision", [rgb("000000", ops(alpha=50390)), rgb("FF0000", ops(alpha=50390))], [[0, 0, 0], [255, 0, 0]]),
    ("scrgb-chain", [scrgb([1000, 1000, 1000], ops(shade=80000)), scrgb([1000, 1000, 1000], '<a:invGamma/>')], [[22, 22, 22], [3, 3, 3]]),
    ("hsl-base-chain", [hsl(0, 100000, 1000, '<a:gamma/>'), hsl(10800000, 75000, 33333, '<a:invGamma/>')], [[39, 0, 0], [2, 76, 76]]),
    ("theme-float-chain", [scheme("accent1", ops(shade=80000)), scheme("accent2", '<a:gamma/>')], [[22, 22, 22], [39, 0, 0]]),
    ("theme-alpha-chain", [scheme("accent1", ops(shade=80000, alphaMod=125000)), scheme("accent2", '<a:gamma/>')], [[22, 22, 22], [39, 0, 0]]),
]


def themed(entries, alpha):
    """A complete slide → layout → master → theme relationship chain."""
    ns = "http://schemas.openxmlformats.org/"
    rel = ns + "officeDocument/2006/relationships/"
    relationship = lambda values: '<Relationships xmlns="' + ns + 'package/2006/relationships">' + "".join(f'<Relationship Id="rId{i}" Type="{rel}{kind}" Target="{target}"/>' for i, (kind, target) in enumerate(values, 1)) + '</Relationships>'
    group = '<p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree>'
    decl = f'xmlns:p="{ns}presentationml/2006/main" xmlns:a="{ns}drawingml/2006/main" xmlns:r="{ns}officeDocument/2006/relationships"'
    mapping = 'bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"'
    entries['ppt/presentation.xml'] = entries['ppt/presentation.xml'].replace('<p:sldIdLst>', '<p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId2"/></p:sldMasterIdLst><p:sldIdLst>')
    entries['ppt/_rels/presentation.xml.rels'] = relationship([('slide', 'slides/slide1.xml'), ('slideMaster', 'slideMasters/slideMaster1.xml')])
    entries['ppt/slides/_rels/slide1.xml.rels'] = relationship([('slideLayout', '../slideLayouts/slideLayout1.xml')])
    entries['ppt/slideLayouts/slideLayout1.xml'] = f'<p:sldLayout {decl} type="blank"><p:cSld name="Blank">{group}</p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>'
    entries['ppt/slideLayouts/_rels/slideLayout1.xml.rels'] = relationship([('slideMaster', '../slideMasters/slideMaster1.xml')])
    entries['ppt/slideMasters/slideMaster1.xml'] = f'<p:sldMaster {decl}><p:cSld>{group}</p:cSld><p:clrMap {mapping}/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId2"/></p:sldLayoutIdLst><p:txStyles/></p:sldMaster>'
    entries['ppt/slideMasters/_rels/slideMaster1.xml.rels'] = relationship([('theme', '../theme/theme1.xml'), ('slideLayout', '../slideLayouts/slideLayout1.xml')])
    colors = {'dk1': rgb('000000'), 'lt1': rgb('FFFFFF'), 'dk2': rgb('000000'), 'lt2': rgb('FFFFFF'), 'accent1': scrgb([1000]*3, ops(alpha=40000) if alpha else ''), 'accent2': hsl(0, 100000, 1000), 'accent3': rgb('000000'), 'accent4': rgb('000000'), 'accent5': rgb('000000'), 'accent6': rgb('000000'), 'hlink': rgb('0000FF'), 'folHlink': rgb('800080')}
    colors = ''.join(f'<a:{slot}>{value}</a:{slot}>' for slot, value in colors.items())
    fills = '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>' * 3
    lines = '<a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>' * 3
    effects = '<a:effectStyle><a:effectLst/></a:effectStyle>' * 3
    entries['ppt/theme/theme1.xml'] = f'<a:theme xmlns:a="{ns}drawingml/2006/main" name="Precision"><a:themeElements><a:clrScheme name="Precision">{colors}</a:clrScheme><a:fontScheme name="Empty"><a:majorFont><a:latin typeface=""/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface=""/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="Empty"><a:fillStyleLst>{fills}</a:fillStyleLst><a:lnStyleLst>{lines}</a:lnStyleLst><a:effectStyleLst>{effects}</a:effectStyleLst><a:bgFillStyleLst>{fills}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>'
    for part, kind in [('slideLayouts/slideLayout1.xml', 'slideLayout'), ('slideMasters/slideMaster1.xml', 'slideMaster'), ('theme/theme1.xml', 'theme')]:
        entries['[Content_Types].xml'] = entries['[Content_Types].xml'].replace('</Types>', f'<Override PartName="/ppt/{part}" ContentType="application/vnd.openxmlformats-officedocument.{"theme" if kind == "theme" else "presentationml." + kind}+xml"/></Types>')


def generate(directory):
    goldens = []
    originals = [rgb('FFFFFF', ops(shade=50000)), rgb('000000', ops(tint=50000))]
    for name, sources, expected in CASES:
        for variant in ['transformed', 'precise', 'base']:
            entries = ENTRIES.copy()
            if name.startswith('theme-'):
                themed(entries, name == 'theme-alpha-chain')
            colors = sources if variant == 'transformed' else [rgb(''.join(f'{c:02X}' for c in value), ops(alpha=50390) if name == 'alpha-precision' else ops(alpha=50000) if name == 'theme-alpha-chain' and i == 0 else '') for i, value in enumerate(expected)] if variant == 'precise' else [rgb('123456'), rgb('808080')]
            for original, color in zip(originals, colors):
                entries['ppt/slides/slide1.xml'] = entries['ppt/slides/slide1.xml'].replace(original, color)
            with ZipFile(directory / f'colors-pipeline-{name}-{variant}.pptx', 'w') as package:
                for part, content in entries.items():
                    info = ZipInfo(part, date_time=(2000, 1, 1, 0, 0, 0))
                    info.compress_type = ZIP_DEFLATED
                    info.external_attr = 0o100644 << 16
                    package.writestr(info, content)
        goldens.append({'name': name, 'rgb': expected, 'pixels': [[127]*3, [255, 127, 127]] if name == 'alpha-precision' else [[138]*3, expected[1]] if name == 'theme-alpha-chain' else expected})
    (directory / 'color-pipeline-goldens.json').write_text(json.dumps(goldens, indent=2) + '\n')


if __name__ == '__main__':
    generate(Path(__file__).resolve().parent)
