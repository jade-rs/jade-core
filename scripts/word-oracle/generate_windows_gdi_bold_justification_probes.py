"""Original Word controls for GDI synthetic-bold cells in legacy mixed justification."""
from pathlib import Path
from io import BytesIO
from zipfile import ZipFile,ZipInfo,ZIP_DEFLATED
import hashlib,json
from lxml import etree as E
ROOT=Path(__file__).resolve().parents[2]
REGISTRY=Path('tests/word-oracle/windows/legacy-justification-observations.json')
REGISTRY_SHA256='2f0cba4117165e39fac724f4232216072263697861aa958072937c3051aabe31'
q=lambda n:'{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'+n

def generate(root=ROOT):
 registry=(root/REGISTRY).read_bytes()
 if hashlib.sha256(registry).hexdigest()!=REGISTRY_SHA256:raise ValueError('Original parent registry changed')
 parents={r['name']:r for r in json.loads(registry)['fixtures']}
 for family,label in [('Times New Roman','times'),('Arial','arial')]:
  row=parents['word-windows-legacy-font-arial-bold-s12-m14-manual-w8504' if label=='arial' else 'word-windows-legacy-font-times-s12-m14-manual-w8504']
  source=(root/row['source']['file']).read_bytes()
  if hashlib.sha256(source).hexdigest()!=row['source']['sha256']:raise ValueError('Original parent bytes changed')
  for style,italic in [('bold',False),('bold-italic',True)]:
   for mode in (12,14):
    for balance in (False,True):
     for width in (3000,8504):
      with ZipFile(BytesIO(source)) as package:
       doc=E.fromstring(package.read('word/document.xml'));settings=E.fromstring(package.read('word/settings.xml'))
       paragraph,=doc.iter(q('p'));paragraph.find(q('pPr')).find(q('ind')).set(q('right'),str(8504-width))
       run,=doc.iter(q('r'));props=run.find(q('rPr'));fonts=props.find(q('rFonts'))
       for slot in ('ascii','hAnsi','cs'):fonts.set(q(slot),family)
       fonts.set(q('eastAsia'),'MS Mincho')
       for tag,enabled in [('b',True),('i',italic)]:
        node=props.find(q(tag))
        if node is None:node=E.SubElement(props,q(tag))
        node.set(q('val'),'1' if enabled else '0')
       b,=settings.iter(q('balanceSingleByteDoubleByteWidth'));b.set(q('val'),'1' if balance else '0')
       mode_node,=settings.xpath("//w:compatSetting[@w:name='compatibilityMode']",namespaces={'w':q('')[1:-1]});mode_node.set(q('val'),str(mode))
       parts={n:E.tostring(x,encoding='utf-8',xml_declaration=True,standalone=True) for n,x in [('word/document.xml',doc),('word/settings.xml',settings)]}
       output=BytesIO()
       with ZipFile(output,'w',compression=ZIP_DEFLATED) as target:
        for item in package.infolist():
         info=ZipInfo(item.filename,(2026,11,19,0,0,0));info.create_system=0;info.compress_type=ZIP_DEFLATED;info.external_attr=item.external_attr
         target.writestr(info,parts.get(item.filename,package.read(item.filename)))
      name=f'word-windows-gdi-bold-justify-{label}-{style}-s12-m{mode}-balance{int(balance)}-manual-w{width}'
      facts=dict(derived_parent=row['source'],parent_name=row['name'],font_family=family,east_asia_font_family='MS Mincho',font_style=style,font_size_pt=12,compatibility_mode=mode,balance_single_byte_double_byte_width=balance,line_ending='manual',line_width_twips=width,text='Ag日本語 ABC Ag日本語',following_text='X',expected_non_whitespace_glyphs=14,changed_parts=list(parts))
      yield name,output.getvalue(),facts
