from pathlib import Path
from io import BytesIO
import copy,hashlib,json,tempfile,unittest,uuid
from zipfile import ZipFile
from unittest.mock import patch
import os
import windows_xps_fonts as cache_fonts
from fontTools.ttLib import TTFont
from check_platform_coordinates import ROOT,read_json
from windows_xps_fonts import extract_xps_fonts,font_covers

class OriginalXpsFonts(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        index=read_json(ROOT/'tests/word-oracle/windows/legacy-justification-observations.json')
        cls.row=next(r for r in index['fixtures'] if r['name']=='word-windows-legacy-font-times-s12-m15-manual-w3000')
        cls.manifest=ROOT/cls.row['capture']['file'];capture=read_json(cls.manifest)
        cls.document=next(d for d in capture['documents'] if d['name']==cls.row['name'])
        cls.xps=cls.manifest.parent/cls.document['xps']
        cls.decoded={}
        with ZipFile(cls.xps) as package:
            for member in package.namelist():
                if member.lower().endswith('.odttf'):
                    data=bytearray(package.read(member));key=uuid.UUID(Path(member).stem).bytes[::-1]
                    for i in range(32):data[i]^=key[i%16]
                    cls.decoded[member]=bytes(data)

    def test_original_fonts_are_byte_exact_and_provenance_binds_word_capture(self):
        with tempfile.TemporaryDirectory() as temporary:
            target=Path(temporary)/'fonts'
            facts=extract_xps_fonts(self.xps,target)
            proof=read_json(target/'xps-font-provenance.json')
            self.assertEqual(proof['source_sha256'],self.document['xps_sha256'])
            self.assertEqual(len(facts),len(self.decoded))
            for fact in facts:
                self.assertEqual(hashlib.sha256(self.decoded[fact['resource']]).hexdigest(),fact['sha256'])
                if fact['selected']:
                    self.assertEqual((target/(fact['sha256']+'.ttf')).read_bytes(),self.decoded[fact['resource']])
            self.assertEqual(sum(not f['selected'] for f in facts),1)
            times=next(f for f in facts if f['selected'] and f['family']=='Times New Roman')
            self.assertEqual(times['version'],'Version 7.13')
            self.assertIn('kern',times['tables'])
            self.assertIn('GPOS',times['tables'])

    def pair(self):
        faces=[TTFont(BytesIO(data)) for data in self.decoded.values()]
        self.addCleanup(lambda:[f.close() for f in faces])
        return next((a,b) for a in faces for b in faces if a is not b and font_covers(a,b))

    def test_only_proven_subsets_are_omitted(self):
        complete,subset=self.pair()
        self.assertFalse(font_covers(subset,complete))
        self.assertFalse(font_covers(complete,complete))
        for field in ('usWeightClass','xAvgCharWidth'):
            changed=copy.deepcopy(subset);setattr(changed['OS/2'],field,getattr(changed['OS/2'],field)+1)
            self.assertFalse(font_covers(complete,changed))
        for field in ('macStyle','unitsPerEm','fontRevision'):
            changed=copy.deepcopy(subset);setattr(changed['head'],field,getattr(changed['head'],field)+1)
            self.assertFalse(font_covers(complete,changed))
        for name_id in (1,5,6):
            changed=copy.deepcopy(subset)
            for entry in changed['name'].names:
                if entry.nameID==name_id:entry.string='Different'.encode(entry.getEncoding())
            self.assertFalse(font_covers(complete,changed))
        changed=copy.deepcopy(subset);glyph=next(iter(changed.getBestCmap().values()))
        width,bearing=changed['hmtx'][glyph];changed['hmtx'][glyph]=(width+1,bearing)
        self.assertFalse(font_covers(complete,changed))
        changed=copy.deepcopy(subset)
        glyph=next(g for g in changed.getBestCmap().values() if changed['glyf'][g].numberOfContours>0)
        x,y=changed['glyf'][glyph].coordinates[0]
        changed['glyf'][glyph].coordinates[0]=(x+1,y)
        self.assertFalse(font_covers(complete,changed))
        changed=copy.deepcopy(subset)
        for table in changed['cmap'].tables:
            if table.isUnicode():table.cmap={}
        self.assertFalse(font_covers(complete,changed))
        changed=copy.deepcopy(subset)
        changed['GPOS'].table.Version+=1
        self.assertFalse(font_covers(complete,changed))

    def test_invalid_font_resources_fail_without_guessing_a_guid_order(self):
        for member,data in [('Resources/bad.odttf',b'x'*40),('Resources/'+str(uuid.UUID(int=0))+'.odttf',b'x'*10),('Resources/plain.ttf',b'badfont')]:
            with self.subTest(member=member),tempfile.TemporaryDirectory() as temporary:
                p=Path(temporary);source=p/'bad.xps'
                with ZipFile(source,'w') as package:package.writestr(member,data)
                with self.assertRaises(ValueError):extract_xps_fonts(source,p/'fonts')

    def test_cache_cannot_replace_original_font_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            p=Path(temporary);facts=extract_xps_fonts(self.xps,p/'first',p/'cache')
            selected=next(f for f in facts if f['selected'])
            cached=p/'cache'/(selected['sha256']+'.ttf')
            cached.write_bytes(b'corrupt')
            with self.assertRaises(ValueError):extract_xps_fonts(self.xps,p/'second',p/'cache')

    def test_cache_rotates_groups_before_the_filesystem_link_limit(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(cache_fonts, '_CACHE_LINK_LIMIT', 2):
            root=Path(temporary);cache=root/'cache'
            first=extract_xps_fonts(self.xps,root/'first',cache)
            for name in ('second','third'):
                self.assertEqual(extract_xps_fonts(self.xps,root/name,cache),first)
                for fact in first:
                    if fact['selected']:
                        self.assertEqual((root/name/(fact['sha256']+'.ttf')).read_bytes(),self.decoded[fact['resource']])
            expected={f['sha256'] for f in first if f['selected']}
            self.assertEqual(len(list(cache.glob('*.ttf'))),len(expected)*3)
            self.assertTrue(all(p.stat().st_nlink<=2 for p in cache.glob('*.ttf')))

    def test_ntfs_link_limit_race_uses_another_byte_identical_group(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);cache=root/'cache';cache.mkdir();destination=root/'font.ttf'
            data=b'Original font program';digest=hashlib.sha256(data).hexdigest();real=os.link;rejected=[]
            def link(source,target):
                if Path(target)==destination and Path(source).name==digest+'.ttf':
                    error=OSError('NTFS link count raced');error.winerror=1142;rejected.append(str(source));raise error
                return real(source,target)
            with patch.object(cache_fonts.os,'link',side_effect=link):
                cache_fonts.link_cached_font(data,digest,cache,destination)
            self.assertEqual(len(rejected),1)
            self.assertEqual(destination.read_bytes(),data)
            self.assertEqual((cache/(digest+'.ttf')).read_bytes(),data)
            self.assertTrue((cache/(digest+'.group-1.ttf')).exists())

    @unittest.skipUnless(os.name == 'nt', 'Windows CreateHardLinkW long-path regression')
    def test_long_versioned_fixture_path_keeps_the_original_font_hardlink(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);cache=root/'cache';cache.mkdir()
            parent=root/('fixture-'+'a'*82)/('version-'+'b'*82)
            parent.mkdir(parents=True)
            directory=parent/'fonts'
            try:
                facts=extract_xps_fonts(self.xps,directory,cache)
                registered=[]
                for fact in facts:
                    if not fact['selected']:continue
                    destination=directory/(fact['sha256']+'.ttf')
                    self.assertGreaterEqual(len(str(destination.resolve())),260)
                    actual=cache_fonts.font_io_path(destination)
                    self.assertEqual(actual.read_bytes(),self.decoded[fact['resource']])
                    cached=cache/(fact['sha256']+'.ttf')
                    self.assertTrue(os.path.samefile(actual,cached))
                    self.assertEqual(cached.stat().st_nlink,2)
                    registered.append(dict(path=str(destination)))
                from check_windows_font_coordinates import require_original_font_programs_registered
                require_original_font_programs_registered(directory,dict(fonts=registered))
            finally:
                if directory.exists():
                    for entry in directory.iterdir():cache_fonts.font_io_path(entry).unlink()

    def test_unrelated_link_errors_are_not_hidden_by_cache_rotation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);cache=root/'cache';cache.mkdir();destination=root/'font.ttf'
            data=b'Original font program';digest=hashlib.sha256(data).hexdigest();real=os.link
            def link(source,target):
                if Path(target)==destination:raise PermissionError(13,'Denied')
                return real(source,target)
            with patch.object(cache_fonts.os,'link',side_effect=link),self.assertRaises(PermissionError):
                cache_fonts.link_cached_font(data,digest,cache,destination)
            self.assertFalse(destination.exists())

if __name__=='__main__':unittest.main()
