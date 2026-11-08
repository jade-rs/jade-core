from pathlib import Path
import zipfile, json
out=Path(__file__).parent
W='http://schemas.openxmlformats.org/wordprocessingml/2006/main'
font='<w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="游明朝"/><w:sz w:val="24"/>'
def run(t,layout=''):
 return f'<w:r><w:rPr>{font}{layout}</w:rPr><w:t>{t}</w:t></w:r>'
def section(direction):
 return f'<w:sectPr><w:type w:val="nextPage"/><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/><w:textDirection w:val="{direction}"/></w:sectPr>'
body=''; cases=[]
for direction in ['tbRl','tbRlV']:
 for compress in [None,'0','1']:
  attrs='' if compress is None else f' w:vertCompress="{compress}"'
  text=run('前')+run('12',f'<w:eastAsianLayout w:id="1" w:vert="1"{attrs}/>')+run('年')+run('1234567890',f'<w:eastAsianLayout w:id="2" w:vert="1"{attrs}/>')+run('後')
  body+=f'<w:p><w:pPr><w:spacing w:before="0" w:after="0"/></w:pPr>{text}</w:p>'
  cases.append({'page':len(cases)+1,'direction':direction,'vertCompress':compress})
  if len(cases)<6: body+=f'<w:p><w:pPr>{section(direction)}</w:pPr></w:p>'
body+=section('tbRlV')
ct='<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>'
rels='<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>'
with zipfile.ZipFile(out/'east-asian-compress.docx','w',zipfile.ZIP_DEFLATED) as z:
 z.writestr('[Content_Types].xml',ct);z.writestr('_rels/.rels',rels)
 z.writestr('word/document.xml',f'<w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>')
(out/'cases.json').write_text(json.dumps(cases,indent=2))
