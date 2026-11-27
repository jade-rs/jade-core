"""Assert percentage-table origins against immutable, versioned Windows Word output."""
import argparse
import hashlib
from pathlib import Path
import subprocess
import sys
from zipfile import ZipFile

from check_platform_coordinates import (
    ROOT, checked_file, compare, extract_fonts, pdf_origins, read_json, require,
)
from check_windows_font_coordinates import (
    has_visible_vector_ink, require_original_font_programs_registered,
)
from extract_windows_emf import extract
from generate_windows_percentage_table_probes import PARENT, PARENT_SHA256

INDEX=Path('tests/word-oracle/windows/percentage-table-observations.json')
INDEX_SHA256='649749562209cf6b102cc47b49f8b5545d9ec0596c337d165531513c60309435'
TOTAL_GLYPHS=10368


def assert_outputs(drawing, word_visible, word_full, native, screen, painted_visible, painted_full, label):
    compare(drawing,native,label+'/EMF-with-export')
    compare(drawing,screen,label+'/EMF-without-export')
    compare(word_full,native,label+'/logical-PDF-unclipped')
    compare(word_visible,screen,label+'/logical-PDF-visible')
    compare(word_visible,painted_visible,label+'/painted-PDF-visible')
    compare(word_full,painted_full,label+'/painted-PDF-unclipped')


def verify(root=ROOT):
    require(hashlib.sha256((root/INDEX).read_bytes()).hexdigest()==INDEX_SHA256,
            'Percentage-table reference coverage changed')
    index=read_json(root/INDEX)
    require(index['schema']==1 and len(index['fixtures'])==108,'Percentage-table coverage removed')
    parent_manifest=read_json(checked_file(root,index['mac_parent_manifest']))
    require(parent_manifest['reference']==index['mac_parent_reference']
            and parent_manifest['reference']['platform']=='mac'
            and parent_manifest['reference']['version']=='16.114.3','Original Mac reference changed')
    require(hashlib.sha256((root/PARENT).read_bytes()).hexdigest()==PARENT_SHA256
            ==parent_manifest['sourceSha256'],'Original Mac source changed')
    cases=[];names=set()
    for case in index['fixtures']:
        name=case['name'];require(name not in names,'Duplicate percentage-table case');names.add(name)
        require(case['coordinate_gate_promotion']=='percentage-tables','Percentage-table gate removed')
        source=checked_file(root,case['source']);public=checked_file(root,case['public_source'])
        require(source.read_bytes()==public.read_bytes(),'Public percentage-table input changed')
        manifest=checked_file(root,case['capture']);capture=read_json(manifest)
        require(capture['status']=='complete' and capture['platform']=='windows','Incomplete Word capture')
        require(capture['word']==case['word'] and capture['capture_id']==case['capture_id'],
                'Original Word version/capture identity changed')
        require(case['word']['product_version']=='16.0.19127.20936'
                and case['word']['executable_sha256']=='7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312',
                'Recorded Word executable/version changed')
        document,=capture['documents'];facts=case['input_specification']
        require(document['name']==name and document['sha256']==case['source']['sha256'],'Wrong Word input')
        require(document['open_mode']=='normal-read-only','Word input was repaired or converted')
        require(document['compatibility_mode']==facts['word_compatibility_mode']
                and facts['parent_source']==PARENT and facts['parent_sha256']==PARENT_SHA256,'Wrong source/mode')
        archive=checked_file(root,case['archive'])
        with ZipFile(archive) as bundle:
            require(bundle.read('capture.json')==manifest.read_bytes(),'Archived Word metadata changed')
            require(bundle.read('inputs/'+name+'.docx')==source.read_bytes(),'Archived Word input changed')
        original=manifest.parent/document['pdf']
        require(hashlib.sha256(original.read_bytes()).hexdigest()==document['pdf_sha256'],'Original Word PDF changed')
        for page in document['metafiles']:
            require(hashlib.sha256((manifest.parent/page['file']).read_bytes()).hexdigest()==page['sha256'],
                    'Original Word EMF changed')
        drawing,=extract(manifest)['fixtures'];visible=pdf_origins(original);full=pdf_origins(original,True)
        require(drawing['page_count']==document['pages']==1,'Percentage-table page coverage changed')
        require(len(drawing['glyphs'])==case['glyph_count'],'Percentage-table glyph coverage changed')
        compare(full,drawing,name+'/original-Word-screen-PDF')
        compare(full,visible,name+'/original-Word-visible-unclipped')
        require(not has_visible_vector_ink(original),'Original Word percentage-table paint changed')
        cases.append((case,source,original,drawing,visible,full))
    require(sum(c[0]['glyph_count'] for c in cases)==int(TOTAL_GLYPHS),'Percentage-table origin coverage removed')
    return cases


def check(executable,output,root=ROOT):
    require(sys.platform=='win32','Windows percentage-table checks require Windows')
    cases=verify(root);output.mkdir(parents=True,exist_ok=False)
    for case,source,original,drawing,visible,full in cases:
        target=output/case['name'];target.mkdir();fonts=target/'fonts'
        extract_fonts(original,fonts);options=['--fonts-directory',str(fonts)]
        actual,screen,painted=target/'native.json',target/'screen.json',target/'native.pdf'
        subprocess.run([str(executable),str(source),str(actual),'windows',str(painted),'--unclipped-tables',*options],check=True)
        subprocess.run([str(executable),str(source),str(screen),'windows',*options],check=True)
        native,without_export=read_json(actual),read_json(screen)
        for model in (native,without_export):
            require(model['input_sha256']==case['source']['sha256'],'Wrong rendered percentage-table input')
            require_original_font_programs_registered(fonts,model)
        assert_outputs(drawing,visible,full,native,without_export,
                       pdf_origins(painted),pdf_origins(painted,True),case['name'])
        require(actual.with_suffix('.png').read_bytes()==screen.with_suffix('.png').read_bytes(),
                case['name']+': PDF export changed screen pixels')
        require(painted.read_bytes()!=original.read_bytes(),'Word self-comparison')
        require(not has_visible_vector_ink(painted),case['name']+': native PDF contains extra guide/border paint')
        print(case['name'],case['word']['product_version'],'six origin assertions and preserved screen: PASS',flush=True)
    return len(cases)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable',type=Path)
    parser.add_argument('--output-directory',type=Path,default=ROOT/'target/windows-percentage-table-coordinates')
    args=parser.parse_args()
    if args.native_executable:
        print('Checked',check(args.native_executable.resolve(),args.output_directory.resolve()),'Windows percentage-table cases')
    else:print('Verified',len(verify()),'versioned original Word percentage-table cases')
