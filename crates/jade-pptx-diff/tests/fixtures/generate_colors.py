"""Generate minimal font-free DrawingML tint/shade regression fixtures."""
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

ENTRIES = {'[Content_Types].xml': '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/></Types>', '_rels/.rels': '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/></Relationships>', 'ppt/presentation.xml': '<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>', 'ppt/_rels/presentation.xml.rels': '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>', 'ppt/slides/slide1.xml': '<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr><p:sp><p:nvSpPr><p:cNvPr id="2" name="color-0"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="508000" y="508000"/><a:ext cx="1524000" cy="1524000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="FFFFFF"><a:shade val="50000"/></a:srgbClr></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="color-1"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="2540000" y="508000"/><a:ext cx="1524000" cy="1524000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="000000"><a:tint val="50000"/></a:srgbClr></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr></p:sp><p:sp><p:nvSpPr><p:cNvPr id="4" name="color-2"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="508000" y="2540000"/><a:ext cx="1524000" cy="1524000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="808080"/></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr></p:sp><p:sp><p:nvSpPr><p:cNvPr id="5" name="color-3"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="2540000" y="2540000"/><a:ext cx="1524000" cy="1524000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="BCBCBC"/></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr></p:sp></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>'}


def generate(directory: Path):
    for name, replacement in [("colors-transformed.pptx", None), ("colors-linear.pptx", "BCBCBC"), ("colors-wrong-srgb.pptx", "808080")]:
        with ZipFile(directory / name, "w") as z:
            for part, text in ENTRIES.items():
                if part == "ppt/slides/slide1.xml" and replacement:
                    for old in ['<a:srgbClr val="FFFFFF"><a:shade val="50000"/></a:srgbClr>', '<a:srgbClr val="000000"><a:tint val="50000"/></a:srgbClr>']:
                        text = text.replace(old, f'<a:srgbClr val="{replacement}"/>')
                info = ZipInfo(part, date_time=(2000, 1, 1, 0, 0, 0))
                info.compress_type = ZIP_DEFLATED
                info.external_attr = 0o100644 << 16
                z.writestr(info, text)


    chains = [
        '<a:srgbClr val="FFFFFF"><a:shade val="20000"/><a:shade val="20000"/></a:srgbClr>',
        '<a:srgbClr val="333333"><a:shade val="10000"/><a:shade val="50000"/></a:srgbClr>',
    ]
    opacity = [
        '<a:srgbClr val="808080"><a:alphaMod val="50000"/></a:srgbClr>',
        '<a:srgbClr val="BCBCBC"><a:alpha val="40000"/><a:alphaMod val="125000"/></a:srgbClr>',
    ]
    variants = [
        ("chain-transformed", chains),
        ("chain-precise", ['<a:srgbClr val="383838"/>', '<a:srgbClr val="050505"/>']),
        ("chain-rounded", ['<a:srgbClr val="393939"/>', '<a:srgbClr val="060606"/>']),
        ("chain-tint-transformed", ['<a:srgbClr val="808080"><a:tint val="20000"/><a:tint val="20000"/></a:srgbClr>', '<a:srgbClr val="FFFFFF"><a:shade val="20000"/><a:tint val="50000"/></a:srgbClr>']),
        ("chain-tint-precise", ['<a:srgbClr val="FBFBFB"/>', '<a:srgbClr val="CBCBCB"/>']),
        ("chain-tint-rounded", ['<a:srgbClr val="FCFCFC"/>', '<a:srgbClr val="CCCCCC"/>']),
        ("alpha-transformed", opacity),
        ("alpha-explicit", ['<a:srgbClr val="808080"><a:alpha val="50000"/></a:srgbClr>', '<a:srgbClr val="BCBCBC"><a:alpha val="50000"/></a:srgbClr>']),
        ("alpha-opaque", ['<a:srgbClr val="808080"/>', '<a:srgbClr val="BCBCBC"/>']),
        ("chain-alpha-transformed", ['<a:srgbClr val="FFFFFF"><a:shade val="20000"/><a:alphaMod val="50000"/><a:shade val="20000"/></a:srgbClr>', '<a:srgbClr val="333333"><a:shade val="10000"/><a:alpha val="50000"/><a:shade val="50000"/></a:srgbClr>']),
        ("chain-alpha-precise", ['<a:srgbClr val="383838"><a:alpha val="50000"/></a:srgbClr>', '<a:srgbClr val="050505"><a:alpha val="50000"/></a:srgbClr>']),
    ]
    unary = {
        "inv": ("FEFBF4", "E5E5E5"),
        "comp": ("563412", "808080"),
        "gray": ("333333", "808080"),
        "gamma": ("4B7D9D", "BCBCBC"),
        "invGamma": ("020918", "373737"),
    }
    for transform, direct in unary.items():
        variants.extend([
            (f"unary-{transform}-transformed", [f'<a:srgbClr val="{color}"><a:{transform}/></a:srgbClr>' for color in ("123456", "808080")]),
            (f"unary-{transform}-precise", [f'<a:srgbClr val="{color}"/>' for color in direct]),
            (f"unary-{transform}-base", [f'<a:srgbClr val="{color}"/>' for color in ("123456", "808080")]),
        ])
    variants.extend([
        ("unary-ordered-transformed", ['<a:srgbClr val="123456"><a:gamma/><a:invGamma/><a:comp/></a:srgbClr>', '<a:srgbClr val="FFFFFF"><a:shade val="20000"/><a:invGamma/><a:gamma/><a:shade val="20000"/><a:alphaMod val="50000"/></a:srgbClr>']),
        ("unary-ordered-precise", ['<a:srgbClr val="563412"/>', '<a:srgbClr val="383838"><a:alpha val="50000"/></a:srgbClr>']),
    ])
    original = ['<a:srgbClr val="FFFFFF"><a:shade val="50000"/></a:srgbClr>', '<a:srgbClr val="000000"><a:tint val="50000"/></a:srgbClr>']
    for name, palette in variants:
        with ZipFile(directory / f"colors-{name}.pptx", "w") as z:
            for part, text in ENTRIES.items():
                if part == "ppt/slides/slide1.xml":
                    for old, new in zip(original, palette):
                        text = text.replace(old, new)
                    if name in ("chain-alpha-precise", "unary-ordered-precise"):
                        # A separate control changes; the first two swatches
                        # stay equivalent and visible in the comparison view.
                        text = text.replace('<a:srgbClr val="BCBCBC"/>', '<a:srgbClr val="BDBDBD"/>')
                info = ZipInfo(part, date_time=(2000, 1, 1, 0, 0, 0))
                info.compress_type = ZIP_DEFLATED
                info.external_attr = 0o100644 << 16
                z.writestr(info, text)


if __name__ == "__main__":
    generate(Path(__file__).resolve().parent)
