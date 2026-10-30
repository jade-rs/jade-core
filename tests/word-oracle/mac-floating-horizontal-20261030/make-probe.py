#!/usr/bin/env python3
"""Generate paired floating-table probes; export both in Word before comparing.

Usage: python3 make-probe.py SOURCE_FLOATING_ORDER.docx OUTPUT_DIRECTORY
Only compatibilityMode differs between the two generated documents.
"""
import zipfile
import pathlib
import sys
out=pathlib.Path(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)
source=pathlib.Path(sys.argv[1])
W='http://schemas.openxmlformats.org/wordprocessingml/2006/main'
cases=[('ML','margin','left',None),('MC','margin','center',None),('MR','margin','right',None),('PL','page','left',None),('PC','page','center',None),('PR','page','right',None),('MX','margin',None,None),('MRPAD','margin','right',240)]
body='<w:p><w:r><w:t>Horizontal floating table probe</w:t></w:r></w:p>'
for i,(label,anchor,align,padding) in enumerate(cases):
    x=f'w:tblpXSpec="{align}"' if align else 'w:tblpX="0"'
    mar=f'<w:tblCellMar><w:left w:w="60" w:type="dxa"/><w:right w:w="{padding}" w:type="dxa"/></w:tblCellMar>' if padding else ''
    body+=f'<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblpPr w:horzAnchor="{anchor}" w:vertAnchor="page" {x} w:tblpY="{1600+i*1400}"/><w:tblW w:type="dxa" w:w="2160"/><w:tblLayout w:type="fixed"/>{mar}</w:tblPr><w:tblGrid><w:gridCol w:w="2160"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:type="dxa" w:w="2160"/></w:tcPr><w:p><w:pPr><w:jc w:val="left"/></w:pPr><w:r><w:t>{label}</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>'
body+='<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1008" w:right="1152" w:bottom="1008" w:left="1152" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr>'
doc=f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>'
with zipfile.ZipFile(source) as z: parts={n:z.read(n) for n in z.namelist()}
for mode in (14,15):
    path=out/f'word-mac-probe-floating-horizontal-{mode}.docx'
    with zipfile.ZipFile(path,'w',zipfile.ZIP_DEFLATED) as z:
        for n,data in parts.items():
            if n=='word/document.xml': data=doc.encode()
            if n=='word/settings.xml': data=data.replace(b'w:val="14"',f'w:val="{mode}"'.encode())
            z.writestr(n,data)
    print(path)
