"""Assert88 source-form hyphen cases against actual Windows Word vector paint.

Logical layout dumps retain editable discretionary controls. They are not
physical paint traces. This gate compares every visible PDF glyph, using the
original Word font programs, and separately preserves live layout and pixels
across export. Existing screen/EMF gates remain required and unchanged.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
from zipfile import ZipFile

import pymupdf

from check_platform_coordinates import ROOT, checked_file, compare, extract_fonts, read_json, require
from check_windows_font_coordinates import require_original_font_programs_registered
from check_windows_observations import INDEX, INDEX_SHA256
from word_painted_control_origins import painted_origins

METADATA=(
    'tests/word-oracle/windows/explicit-hyphen-observations.json',
    'tests/word-oracle/windows/mixed-hyphen-observations.json',
)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require_independent_paint(word_pdf,native_pdf,label):
    require(word_pdf.read_bytes()!=native_pdf.read_bytes(),label+': Word PDF self-comparison')


def verify(root=ROOT):
    require(sha(root/INDEX)==INDEX_SHA256,'Original observation coverage changed')
    index=read_json(root/INDEX)
    facts={r['file']:r for r in index['font_facts']}
    cases=[];archives={};identities=set()
    for relative,expected_count in zip(METADATA,(60,28)):
        require(relative in facts,'Missing source-form original metadata')
        metadata=read_json(checked_file(root,facts[relative]))
        parent=read_json(checked_file(root,metadata['mac_parent_manifest']))
        require(parent['reference']==metadata['mac_parent_reference']
                and parent['reference']['platform']=='mac','Mac original provenance changed')
        require(len(metadata['fixtures'])==expected_count,'Source-form case coverage changed')
        for row in metadata['fixtures']:
            source=checked_file(root,row['source'])
            require(row['input_specification']['source_parent_sha256']==parent['sourceSha256'],
                    'Source-form control lost its Mac original lineage')
            archive=checked_file(root,row['archive']);manifest=checked_file(root,row['capture'])
            if archive not in archives:
                with ZipFile(archive) as bundle:
                    raw=bundle.read('capture.json')
                    capture=json.loads(raw.decode('utf-8-sig'))
                    require(capture['schema']==2 and capture['status']=='complete'
                            and capture['platform']=='windows','Invalid original Word capture')
                    members={'capture.json'}
                    for doc in capture['documents']:
                        original='inputs/'+doc['name']+'.docx'
                        require(hashlib.sha256(bundle.read(original)).hexdigest()==doc['sha256'],
                                'Archived source changed')
                        members.add(original)
                        outputs=[(doc['pdf'],doc['pdf_sha256'])]+[(p['file'],p['sha256']) for p in doc['metafiles']]
                        require([p['page'] for p in doc['metafiles']]==list(range(1,doc['pages']+1)),
                                'Original Word page coverage changed')
                        for name,digest in outputs:
                            require('/' not in name and '\\' not in name,'Invalid original output path')
                            require(hashlib.sha256(bundle.read(name)).hexdigest()==digest,
                                    'Archived original Word drawing changed')
                            require(sha(manifest.parent/name)==digest,'Expanded original drawing changed')
                            members.add(name)
                    require(len(bundle.namelist())==len(members) and set(bundle.namelist())==members,
                            'Original archive members changed')
                require(raw==manifest.read_bytes(),'Capture metadata differs from original archive')
                archives[archive]=(capture,{d['name']:d for d in capture['documents']})
            capture,documents=archives[archive]
            require(capture['word']==row['word'] and capture['capture_id']==row['capture_id'],
                    'Word executable/version/capture identity changed')
            require(capture['word']['product_version']=='16.0.19127.20936'
                    and capture['word']['executable_sha256']=='7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312',
                    'Unexpected original Word executable version')
            require(row['name'] in documents,'Missing original Word document')
            document=documents[row['name']]
            require(document['sha256']==row['source']['sha256']==row['input_specification']['source_sha256']
                    and document['open_mode']=='normal-read-only','Original source identity changed')
            require(document['compatibility_mode']==row['input_specification']['compatibility_mode'],
                    'Original compatibility mode changed')
            identity=(row['name'],row['source']['sha256'])
            require(identity not in identities,'Duplicate source-form case')
            identities.add(identity)
            pdf=manifest.parent/document['pdf']
            with pymupdf.open(pdf) as original:
                programs={hashlib.sha256(original.extract_font(xref)[3]).hexdigest()
                          for xref in {font[0] for page in original for font in page.get_fonts(full=True)}
                          if original.extract_font(xref)[3]}
            require(programs=={p['program_sha256'] for p in row['original_word_font_programs']},
                    'Original embedded Word font programs changed')
            cases.append((row,source,pdf,painted_origins(pdf)))
    require(len(cases)==len(identities)==88,'Source-form case coverage removed')
    return cases


def check_windows(executable,output,root=ROOT):
    cases=verify(root);output.mkdir(parents=True,exist_ok=True)
    isolation=output/'isolated-localappdata';isolation.mkdir(exist_ok=True)
    environment=dict(os.environ);environment.pop('JADE_OFFICE_FONT_DIR',None)
    environment['LOCALAPPDATA']=str(isolation.resolve())
    executable=executable.resolve();rows=[];failures=[]
    for number,(row,source,word_pdf,expected) in enumerate(cases):
        case=output/f'c{number:02d}';case.mkdir(exist_ok=True)
        fonts=case/'fonts';extract_fonts(word_pdf,fonts)
        native,screen,pdf=(case/name for name in ('native.json','screen.json','native.pdf'))
        with (case/'stderr.log').open('w',encoding='utf-8') as log:
            for path,options in ((native,[str(pdf),'--unclipped-tables']),(screen,[])):
                subprocess.run([str(executable),str(source),str(path),'windows',*options,
                                '--fonts-directory',str(fonts)],check=True,timeout=180,env=environment,
                               stdout=subprocess.DEVNULL,stderr=log)
        require_independent_paint(word_pdf,pdf,row['name'])
        a,b=read_json(native),read_json(screen)
        for model in (a,b):
            require(model['input_sha256']==row['source']['sha256'],'Renderer used different source bytes')
            require_original_font_programs_registered(fonts,model)
        require(a['glyphs']==b['glyphs'] and a['page_count']==b['page_count'],
                row['name']+': export changed logical live geometry')
        require(native.with_suffix('.png').read_bytes()==screen.with_suffix('.png').read_bytes(),
                row['name']+': export changed live pixels')
        actual=painted_origins(pdf);result='PASS'
        try:compare(expected,actual,row['name'])
        except ValueError as error:
            result=str(error);failures.append(dict(name=row['name'],error=result))
        rows.append(dict(name=row['name'],source_sha256=row['source']['sha256'],word=row['word'],
                         capture_id=row['capture_id'],word_pdf_sha256=sha(word_pdf),native_pdf_sha256=sha(pdf),
                         expected_glyphs=len(expected['glyphs']),actual_glyphs=len(actual['glyphs']),
                         verified_word_painted_aliases=expected['verified_painted_aliases'],
                         verified_native_painted_aliases=actual['verified_painted_aliases'],
                         original_font_programs_registered=True,live_geometry_and_pixels_preserved=True,result=result))
        report=dict(scope='Actual vector painted origins, original Word fonts, source and version integrity; live layout/pixels unchanged by export. Logical dumps are not physical screen traces.',
                    executable_sha256=sha(executable),cases=rows,failures=failures)
        (output/'results.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
        print(row['name'],row['word']['product_version'],'actual painted coordinates:',result,flush=True)
    require(len(rows)==88 and not failures,'Windows source-form hyphen painting differs from original Word; see results.json')
    print('All88 actual Windows Word source-form hyphen painted-coordinate cases PASS',flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable',type=Path)
    parser.add_argument('--output-directory',type=Path,default=ROOT/'target/windows-hyphen-paint')
    args=parser.parse_args()
    if args.native_executable:
        check_windows(args.native_executable,args.output_directory)
    else:
        print('Verified',len(verify()),'actual Windows source-form hyphen Word originals')
