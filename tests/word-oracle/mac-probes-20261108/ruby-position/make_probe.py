from pathlib import Path
from zipfile import ZipFile, ZIP_DEFLATED
from xml.sax.saxutils import escape
out=Path(__file__).parent
cases=[('normal',''),('raise2','<w:position w:val="4"/>'),('lower3','<w:position w:val="-6"/>'),('super','<w:vertAlign w:val="superscript"/>'),('sub','<w:vertAlign w:val="subscript"/>'),('super_raise2','<w:vertAlign w:val="superscript"/><w:position w:val="4"/>'),('lower30','<w:position w:val="-60"/>')]
body=''
for label,pr in cases:
 body+=f'<w:p><w:r><w:t>{label}: </w:t></w:r><w:r><w:ruby><w:rubyPr><w:rubyAlign w:val="left"/><w:hps w:val="16"/><w:hpsRaise w:val="30"/><w:hpsBaseText w:val="24"/></w:rubyPr><w:rt><w:r><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="16"/>{pr}</w:rPr><w:t>ABC</w:t></w:r></w:rt><w:rubyBase><w:r><w:rPr><w:rFonts w:ascii="Arial"/><w:sz w:val="24"/></w:rPr><w:t>BASE</w:t></w:r></w:rubyBase></w:ruby></w:r><w:r><w:t> tail</w:t></w:r></w:p>'
 body+='<w:p><w:r><w:t>following line</w:t></w:r></w:p>'
with ZipFile(out/'source-template.docx') as src,ZipFile(out/'ruby-position.docx','w',ZIP_DEFLATED) as dst:
 for name in src.namelist():
  data=src.read(name)
  if name=='word/document.xml':
   text=data.decode(); start=text.index('<w:body>')+len('<w:body>'); end=text.index('<w:sectPr>'); data=(text[:start]+body+text[end:]).encode()
  dst.writestr(name,data)
