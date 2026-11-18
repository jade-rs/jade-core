"""Assert Aptos-size origins against immutable, versioned Windows Word output."""
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
from generate_windows_aptos_size_probes import PARENT, PARENT_SHA256

INDEX=Path('tests/word-oracle/windows/aptos-font-size-observations.json')
INDEX_SHA256='a137c9072de8503becd45ac914d9363e985af64beeae04873843864c5f422ae7'
TOTAL_GLYPHS=13720
TOTAL_SCREEN_GLYPHS=17920
REQUIRED_CASE_NAMES=(
    'word-windows-aptos-size-control-display-bold-size13-compat14',
    'word-windows-aptos-size-control-display-bold-size13-compat15',
    'word-windows-aptos-size-control-display-bold-size13p5-compat14',
    'word-windows-aptos-size-control-display-bold-size13p5-compat15',
    'word-windows-aptos-size-control-display-bold-size14p5-compat14',
    'word-windows-aptos-size-control-display-bold-size14p5-compat15',
    'word-windows-aptos-size-control-display-bold-size15-compat14',
    'word-windows-aptos-size-control-display-bold-size15-compat15',
    'word-windows-aptos-size-control-display-bold-size15p5-compat14',
    'word-windows-aptos-size-control-display-bold-size15p5-compat15',
    'word-windows-aptos-size-control-display-bold-size16-compat14',
    'word-windows-aptos-size-control-display-bold-size16-compat15',
    'word-windows-aptos-size-control-display-bold-size20-compat14',
    'word-windows-aptos-size-control-display-bold-size20-compat15',
    'word-windows-aptos-size-control-display-bolditalic-size13-compat14',
    'word-windows-aptos-size-control-display-bolditalic-size13-compat15',
    'word-windows-aptos-size-control-display-bolditalic-size14-compat14',
    'word-windows-aptos-size-control-display-bolditalic-size15p5-compat14',
    'word-windows-aptos-size-control-display-bolditalic-size15p5-compat15',
    'word-windows-aptos-size-control-display-bolditalic-size16-compat14',
    'word-windows-aptos-size-control-display-bolditalic-size16-compat15',
    'word-windows-aptos-size-control-display-bolditalic-size20-compat14',
    'word-windows-aptos-size-control-display-bolditalic-size20-compat15',
    'word-windows-aptos-size-control-display-italic-size13-compat14',
    'word-windows-aptos-size-control-display-italic-size13-compat15',
    'word-windows-aptos-size-control-display-italic-size13p5-compat14',
    'word-windows-aptos-size-control-display-italic-size13p5-compat15',
    'word-windows-aptos-size-control-display-italic-size14-compat14',
    'word-windows-aptos-size-control-display-italic-size14-compat15',
    'word-windows-aptos-size-control-display-italic-size14p5-compat14',
    'word-windows-aptos-size-control-display-italic-size14p5-compat15',
    'word-windows-aptos-size-control-display-italic-size15-compat14',
    'word-windows-aptos-size-control-display-italic-size15-compat15',
    'word-windows-aptos-size-control-display-italic-size15p5-compat14',
    'word-windows-aptos-size-control-display-italic-size15p5-compat15',
    'word-windows-aptos-size-control-display-italic-size16-compat14',
    'word-windows-aptos-size-control-display-italic-size16-compat15',
    'word-windows-aptos-size-control-display-italic-size20-compat14',
    'word-windows-aptos-size-control-display-italic-size20-compat15',
    'word-windows-aptos-size-control-display-regular-size13-compat14',
    'word-windows-aptos-size-control-display-regular-size13-compat15',
    'word-windows-aptos-size-control-display-regular-size13p5-compat15',
    'word-windows-aptos-size-control-display-regular-size14p5-compat14',
    'word-windows-aptos-size-control-display-regular-size14p5-compat15',
    'word-windows-aptos-size-control-display-regular-size15-compat15',
    'word-windows-aptos-size-control-display-regular-size15p5-compat14',
    'word-windows-aptos-size-control-display-regular-size15p5-compat15',
    'word-windows-aptos-size-control-display-regular-size20-compat14',
    'word-windows-aptos-size-control-display-regular-size20-compat15',
)


def assert_screen_outputs(drawing,native,screen,label):
    compare(drawing,native,label+'/screen-with-export')
    compare(drawing,screen,label+'/screen-without-export')


def assert_outputs(drawing, word_visible, word_full, native, screen, painted_visible, painted_full, label):
    compare(drawing,native,label+'/EMF-with-export')
    compare(drawing,screen,label+'/EMF-without-export')
    compare(word_full,native,label+'/logical-PDF-unclipped')
    compare(word_visible,screen,label+'/logical-PDF-visible')
    compare(word_visible,painted_visible,label+'/painted-PDF-visible')
    compare(word_full,painted_full,label+'/painted-PDF-unclipped')


def verify(root=ROOT):
    require(hashlib.sha256((root/INDEX).read_bytes()).hexdigest()==INDEX_SHA256,
            'Aptos-size reference coverage changed')
    index=read_json(root/INDEX)
    require(index['schema']==1 and len(index['fixtures'])==64,'Aptos-size coverage removed')
    parent_manifest=read_json(checked_file(root,index['mac_parent_manifest']))
    require(parent_manifest['reference']==index['mac_parent_reference']
            and parent_manifest['reference']['platform']=='mac'
            and parent_manifest['reference']['version']=='16.114.3','Original Mac reference changed')
    require(hashlib.sha256((root/PARENT).read_bytes()).hexdigest()==PARENT_SHA256
            ==parent_manifest['sourceSha256'],'Original Mac source changed')
    cases=[];names=set()
    for case in index['fixtures']:
        expected='aptos-sizes' if case['name'] in REQUIRED_CASE_NAMES else 'pending'
        require(case['coordinate_gate_promotion']==expected,'Validated/pending Aptos case identity changed')
        require(case['screen_gate_promotion']=='aptos-size-screen','Required screen gate removed')
    for case in index['fixtures']:
        name=case['name'];require(name not in names,'Duplicate Aptos-size case');names.add(name)
        source=checked_file(root,case['source'])
        if name in REQUIRED_CASE_NAMES:
            public=checked_file(root,case['public_source'])
            require(source.read_bytes()==public.read_bytes(),'Public Aptos-size input changed')
        else:require('public_source' not in case,'Pending PDF case cannot be promoted silently')
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
        require(document['compatibility_mode']==facts['compatibility_mode']
                and facts['mac_parent']==PARENT and facts['mac_parent_sha256']==PARENT_SHA256,'Wrong source/mode')
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
        require(drawing['page_count']==document['pages']==1,'Aptos-size page coverage changed')
        require(len(drawing['glyphs'])==case['glyph_count'],'Aptos-size glyph coverage changed')
        compare(full,visible,name+'/original-Word-visible-unclipped')
        require(not has_visible_vector_ink(original),'Original Word Aptos-size paint changed')
        cases.append((case,source,original,drawing,visible,full))
    require(set(REQUIRED_CASE_NAMES)<=names and len(names)==64,'Aptos screen coverage removed')
    require(sum(c[0]['glyph_count'] for c in cases)==TOTAL_SCREEN_GLYPHS,'Aptos screen origins removed')
    require(sum(c[0]['glyph_count'] for c in cases if c[0]['name'] in REQUIRED_CASE_NAMES)==int(TOTAL_GLYPHS),'Aptos-size origin coverage removed')
    return cases


def check(executable,output,root=ROOT):
    require(sys.platform=='win32','Windows Aptos-size checks require Windows')
    cases=verify(root);output.mkdir(parents=True,exist_ok=False)
    for case,source,original,drawing,visible,full in cases:
        target=output/case['name'];target.mkdir();fonts=target/'fonts'
        extract_fonts(original,fonts);options=['--fonts-directory',str(fonts)]
        actual,screen,painted=target/'native.json',target/'screen.json',target/'native.pdf'
        subprocess.run([str(executable),str(source),str(actual),'windows',str(painted),'--unclipped-tables',*options],check=True)
        subprocess.run([str(executable),str(source),str(screen),'windows',*options],check=True)
        native,without_export=read_json(actual),read_json(screen)
        for model in (native,without_export):
            require(model['input_sha256']==case['source']['sha256'],'Wrong rendered Aptos-size input')
            require_original_font_programs_registered(fonts,model)
        assert_screen_outputs(drawing,native,without_export,case['name'])
        if case['name'] in REQUIRED_CASE_NAMES:
            assert_outputs(drawing,visible,full,native,without_export,
                           pdf_origins(painted),pdf_origins(painted,True),case['name'])
        require(actual.with_suffix('.png').read_bytes()==screen.with_suffix('.png').read_bytes(),
                case['name']+': PDF export changed screen pixels')
        require(painted.read_bytes()!=original.read_bytes(),'Word self-comparison')
        require(not has_visible_vector_ink(painted),case['name']+': native PDF contains extra guide/border paint')
        status='screen + logical/painted PDF: PASS' if case['name'] in REQUIRED_CASE_NAMES else 'screen: PASS; PDF origins pending'
        print(case['name'],case['word']['product_version'],status,flush=True)
    return len(cases)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native-executable',type=Path)
    parser.add_argument('--output-directory',type=Path,default=ROOT/'target/windows-Aptos-size-coordinates')
    args=parser.parse_args()
    if args.native_executable:
        print('Checked',check(args.native_executable.resolve(),args.output_directory.resolve()),'Windows Aptos-size screen cases; 49 full-PDF cases')
    else:print('Verified',len(verify()),'versioned original Word Aptos-size cases')
