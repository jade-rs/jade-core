"""Generate the note/font/size/balance/compatibility Windows Word matrix."""
import hashlib
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED
import re
core=Path(__file__).resolve().parents[2]
source = core / 'public/fixtures/footnote.docx'
if hashlib.sha256(source.read_bytes()).hexdigest() != 'eecd3c87d245794afb8a91116647fb8a79651d39343156d76eb82bb6618badc1':
    raise ValueError('Original footnote source changed')
with ZipFile(core/'public/fixtures/footnote.docx') as z:parts={name:z.read(name) for name in z.namelist()}
for slug,family in [('century','Century'),('arial','Arial'),('yu-mincho','Yu Mincho')]:
 for balance in (0,1):
  for mode in (12,15):
   for size in (21,40):
    target=core/'public/fixtures'/f'word-windows-edge-note-separator-{slug}-balance{balance}-compat{mode}-size{size}.docx'
    with ZipFile(target,'w',compression=ZIP_DEFLATED,compresslevel=9) as z:
     for name,data in sorted(parts.items()):
      if name in ('word/styles.xml','word/document.xml','word/footnotes.xml'):
       text=data.decode('utf-8');text=re.sub(r'<w:rFonts\b[^>]*/>',f'<w:rFonts w:ascii="{family}" w:hAnsi="{family}" w:eastAsia="{family}" w:cs="{family}"/>',text)
       text=re.sub(r'<w:sz w:val="[0-9]+"/>',f'<w:sz w:val="{size}"/>',text);data=text.encode()
      if name=='word/settings.xml':
       text=data.decode('utf-8');text=re.sub(r'<w:balanceSingleByteDoubleByteWidth\b[^>]*/>',f'<w:balanceSingleByteDoubleByteWidth w:val="{balance}"/>',text)
       text=re.sub(r'(<w:compatSetting w:name="compatibilityMode"[^>]*w:val=")[0-9]+',lambda m:m[1]+str(mode),text);data=text.encode()
      info=ZipInfo(name,(2000,1,1,0,0,0));info.compress_type=ZIP_DEFLATED;z.writestr(info,data)
print('Generated24 note/font/size/balance/mode probes')
