"""Check independent Word inputs, font provenance and cache-registration rejection."""
from io import BytesIO
import hashlib
from pathlib import Path
import tempfile
import unittest
from zipfile import ZipFile

import pymupdf
import generate_windows_office_cloud_probes as generate
from check_platform_coordinates import checked_file, compare, pdf_origins, read_json
from check_windows_office_cloud_fonts import INDEX, require_cached_body_font
from extract_windows_emf import extract


class WindowsOfficeCloudFonts(unittest.TestCase):
    def observations(self):
        path = generate.ROOT / INDEX
        self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), 'bae4d7a6f85404c1d515b98c1d8548155f6f70ccfe89b82fdf51053869d7fa6f')
        return read_json(path)

    def test_eight_exact_inputs_keep_other_original_parts(self):
        cases = {c['name']: c for c in self.observations()['fixtures']}
        generated = dict(generate.probes())
        self.assertEqual(len(cases), 8)
        self.assertEqual(set(cases), set(generated))
        self.assertEqual({c['column_width_pt'] for c in cases.values()}, {120, 240})
        self.assertEqual({c['font_size_pt'] for c in cases.values()}, {12, 20})
        with ZipFile(generate.ROOT / generate.PARENT) as parent:
            for name, raw in generated.items():
                with self.subTest(case=name), ZipFile(BytesIO(raw)) as actual:
                    case = cases[name]
                    self.assertEqual(raw, checked_file(generate.ROOT, case['source']).read_bytes())
                    self.assertFalse(case['baseline_all_planes_pass'])
                    self.assertEqual(actual.namelist(), parent.namelist())
                    self.assertEqual([p for p in parent.namelist() if actual.read(p) != parent.read(p)],
                                     ['word/document.xml'])

    def test_original_word_build_sources_fonts_and_both_planes(self):
        page_counts = []
        for case in self.observations()['fixtures']:
            with self.subTest(case=case['name']):
                manifest = checked_file(generate.ROOT, case['capture'])
                capture = read_json(manifest)
                self.assertEqual(capture['word'], case['word'])
                self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(capture['word']['executable_sha256'],
                                 '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                document, = capture['documents']
                page_counts.append(document['pages'])
                self.assertEqual(document['compatibility_mode'], 14)
                self.assertEqual(document['open_mode'], 'normal-read-only')
                with ZipFile(checked_file(generate.ROOT, case['archive'])) as archive:
                    self.assertEqual(archive.read('capture.json'), manifest.read_bytes())
                    self.assertEqual(archive.read('inputs/' + case['name'] + '.docx'),
                                     checked_file(generate.ROOT, case['source']).read_bytes())
                reference = read_json(checked_file(generate.ROOT, case['drawing_reference']))
                self.assertEqual(reference, extract(manifest))
                drawing, = reference['fixtures']
                original_pdf = manifest.parent / document['pdf']
                self.assertEqual(compare(pdf_origins(original_pdf), drawing, 'Word original planes'),
                                 len(drawing['glyphs']))
                with pymupdf.open(original_pdf) as pdf:
                    for program in case['original_word_font_programs']:
                        name, extension, _, data = pdf.extract_font(program['xref'])
                        self.assertEqual((name, extension), (program['postscript_name'], program['extension']))
                        self.assertEqual(hashlib.sha256(data).hexdigest(), program['sha256'])
        self.assertIn(2, page_counts)

    def test_actual_full_font_versions_are_retained_without_copying_program_files(self):
        programs = self.observations()['actual_system_full_font_programs']
        aptos = [p for p in programs if set(p['legacy_family']) & {'Aptos', 'Aptos Display'}]
        self.assertEqual(len(aptos), 8)
        for program in aptos:
            self.assertEqual(program['version'], ['Version 2.01;O365'])
            self.assertEqual(len(program['sha256']), 64)
            self.assertTrue(program['has_GPOS'])
            self.assertIn('CloudFonts', Path(program['cache_relative_path']).parts)
            self.assertTrue(program['cache_relative_path'].startswith('Microsoft/FontCache/'))
            self.assertNotIn('file', program)

    def test_missing_wrong_or_outside_cache_body_fonts_fail(self):
        case = self.observations()['fixtures'][0]
        program = next(p for p in case['original_word_font_programs']
                       if case['font_family'] in p['legacy_family'] and p['weight'] == 400)
        manifest = checked_file(generate.ROOT, case['capture'])
        document, = read_json(manifest)['documents']
        with pymupdf.open(manifest.parent / document['pdf']) as pdf:
            data = pdf.extract_font(program['xref'])[3]
        with tempfile.TemporaryDirectory(prefix='jade-office-cache-rejection-') as directory:
            root = Path(directory)
            cache = root / 'CloudFonts'
            cache.mkdir()
            path = cache / 'original.ttf'
            path.write_bytes(data)
            face = dict(family=case['font_family'], weight=400, italic=False, path=str(path))
            require_cached_body_font(case, dict(fonts=[face]), cache)
            for change in [dict(family='Arial'), dict(weight=700), dict(italic=True)]:
                with self.subTest(change=change), self.assertRaisesRegex(ValueError, 'Missing cached'):
                    require_cached_body_font(case, dict(fonts=[dict(face, **change)]), cache)
            outside = root / 'outside.ttf'
            outside.write_bytes(data)
            with self.assertRaisesRegex(ValueError, 'outside'):
                require_cached_body_font(case, dict(fonts=[dict(face, path=str(outside))]), cache)
            path.write_bytes(b'wrong program')
            with self.assertRaisesRegex(ValueError, 'differs'):
                require_cached_body_font(case, dict(fonts=[face]), cache)


if __name__ == '__main__':
    unittest.main()
