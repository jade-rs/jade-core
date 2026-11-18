"""Derive percentage/grid/AutoFit controls from an immutable Mac original."""
from copy import deepcopy
import hashlib
from io import BytesIO
from zipfile import ZipFile,ZipInfo,ZIP_DEFLATED
from lxml import etree as ET

PARENT='tests/word-oracle/mac-elsa-synthetic/borderless-table-mac/source.docx'
PARENT_SHA256='219dbbfbdd73599dc6a0caf012199e1dde8a3253bff13764b4990ba7071f261d'
q=lambda name:'{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'+name


def generate(parent_bytes):
    if hashlib.sha256(parent_bytes).hexdigest()!=PARENT_SHA256:
        raise ValueError('Original Mac borderless-table source changed')
    with ZipFile(BytesIO(parent_bytes)) as parent:
        document=ET.fromstring(parent.read('word/document.xml'))
        settings=(ET.fromstring(parent.read('word/settings.xml')) if 'word/settings.xml' in parent.namelist()
                  else ET.Element(q('settings'),nsmap={'w':'http://schemas.openxmlformats.org/wordprocessingml/2006/main'}))
        original_body=document.find(q('body'));paragraphs=original_body.findall(q('p'))
        original_table=original_body.find(q('tbl'));section=original_body.find(q('sectPr'))
        assert original_table is not None and len(paragraphs)>=2
        for layout in ('autofit','fixed'):
            for table_percent in (50,100):
                for grid_percent in (10,30,60):
                    for cell_percent in (10,30,60):
                        for mode in (None,14,15):
                            doc=deepcopy(document);body=doc.find(q('body'));body.clear()
                            body.append(deepcopy(paragraphs[0]))
                            table=deepcopy(original_table);props=table.find(q('tblPr'))
                            width=props.find(q('tblW'));width.set(q('type'),'pct');width.set(q('w'),str(table_percent*50))
                            plan=props.find(q('tblLayout'))
                            if plan is None:
                                plan=ET.Element(q('tblLayout'));margins=props.find(q('tblCellMar'))
                                props.insert(list(props).index(margins),plan) if margins is not None else props.append(plan)
                            plan.set(q('type'),layout)
                            grid=table.find(q('tblGrid'));columns=grid.findall(q('gridCol'));assert len(columns)==2
                            first=round(9026*grid_percent/100);grid_twips=[first,9026-first]
                            for column,twips in zip(columns,grid_twips):column.set(q('w'),str(twips))
                            for row in table.findall(q('tr')):
                                cells=row.findall(q('tc'));assert len(cells)==2
                                for cell,percent in zip(cells,(cell_percent,100-cell_percent)):
                                    width=cell.find(q('tcPr')).find(q('tcW'))
                                    width.set(q('type'),'pct');width.set(q('w'),str(percent*50))
                            body.append(table);body.append(deepcopy(paragraphs[1]));body.append(deepcopy(section))
                            changed={'word/document.xml':ET.tostring(doc,encoding='utf-8',xml_declaration=True)}
                            if mode is not None:
                                config=deepcopy(settings);compat=config.find(q('compat'))
                                if compat is None:compat=ET.SubElement(config,q('compat'))
                                modes=[item for item in compat.findall(q('compatSetting')) if item.get(q('name'))=='compatibilityMode']
                                assert len(modes)<=1
                                setting=modes[0] if modes else ET.SubElement(compat,q('compatSetting'))
                                for name,value in [('name','compatibilityMode'),('uri','http://schemas.microsoft.com/office/word'),('val',str(mode))]:
                                    setting.set(q(name),value)
                                changed['word/settings.xml']=ET.tostring(config,encoding='utf-8',xml_declaration=True)
                                types=ET.fromstring(parent.read('[Content_Types].xml'))
                                namespace='{http://schemas.openxmlformats.org/package/2006/content-types}'
                                if not any(e.get('PartName')=='/word/settings.xml' for e in types):
                                    item=ET.SubElement(types,namespace+'Override')
                                    item.set('PartName','/word/settings.xml')
                                    item.set('ContentType','application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml')
                                changed['[Content_Types].xml']=ET.tostring(types,encoding='utf-8',xml_declaration=True)
                                rels=ET.fromstring(parent.read('word/_rels/document.xml.rels'))
                                reltype='http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings'
                                if not any(e.get('Type')==reltype for e in rels):
                                    ids={e.get('Id') for e in rels};identity='rIdJadeSettings'
                                    while identity in ids:identity+='Next'
                                    item=ET.SubElement(rels,'{http://schemas.openxmlformats.org/package/2006/relationships}Relationship')
                                    item.set('Id',identity);item.set('Type',reltype);item.set('Target','settings.xml')
                                changed['word/_rels/document.xml.rels']=ET.tostring(rels,encoding='utf-8',xml_declaration=True)
                            buffer=BytesIO()
                            with ZipFile(buffer,'w',compression=ZIP_DEFLATED) as output:
                                for entry in parent.infolist():
                                    info=ZipInfo(entry.filename,(2026,11,18,0,0,0));info.create_system=0
                                    info.compress_type=ZIP_DEFLATED;info.external_attr=entry.external_attr
                                    output.writestr(info,changed.get(entry.filename,parent.read(entry.filename)))
                                for part in sorted(set(changed)-set(parent.namelist())):
                                    info=ZipInfo(part,(2026,11,18,0,0,0));info.create_system=0
                                    info.compress_type=ZIP_DEFLATED;info.external_attr=0x01800000
                                    output.writestr(info,changed[part])
                            label='default' if mode is None else str(mode)
                            name=f'word-windows-pct-table-{layout}-table{table_percent}-grid{grid_percent}-cells{cell_percent}-compat{label}'
                            yield name,buffer.getvalue(),dict(parent_source=PARENT,parent_sha256=PARENT_SHA256,
                                layout=layout,table_percent=table_percent,saved_grid_percent=grid_percent,
                                saved_grid_twips=grid_twips,cell_percent=cell_percent,source_compatibility_mode=mode,
                                word_compatibility_mode=12 if mode is None else mode,
                                rows=3,columns=2,cell_horizontal_margins_pt=[0,0],table_rules='nil',changed_parts=sorted(changed))
