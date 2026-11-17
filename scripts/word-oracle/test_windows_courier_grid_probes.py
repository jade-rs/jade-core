"""Keep the new Word originals, source reproduction and platform provenance strict."""
import hashlib
from io import BytesIO
import unittest
from zipfile import ZipFile

from lxml import etree as ET

from check_platform_coordinates import ROOT, checked_file, read_json
from generate_windows_courier_grid_probes import MC, PARENT, W14, probes


class CourierGridProbes(unittest.TestCase):
    def test_every_input_reproduces_exact_original_word_bytes(self):
        generated = dict(probes())
        self.assertEqual(len(generated), 34)
        cases = {r['name']: r for r in read_json(
            ROOT / 'tests/word-oracle/windows/font-coordinates.json')['fixtures']}
        for name, data in generated.items():
            with self.subTest(name=name):
                source = checked_file(ROOT, cases[name]['source'])
                self.assertEqual(data, source.read_bytes())
                self.assertEqual(hashlib.sha256(data).hexdigest(), cases[name]['source']['sha256'])

    def test_original_mac_source_and_untouched_parts_are_preserved(self):
        source = ROOT / PARENT
        original = source.read_bytes()
        self.assertEqual(hashlib.sha256(original).hexdigest(),
                         '027b0eed0c02e0daf5eb3303b3a6c419f5e5ed3cbc5abdd228ad69d31e35f5c4')
        manifest = read_json(source.with_name('manifest.json'))
        self.assertEqual(manifest['reference']['platform'], 'mac')
        self.assertEqual(manifest['sourceSha256'], hashlib.sha256(original).hexdigest())
        self.assertEqual(manifest['coordinatesSha256'], hashlib.sha256(
            source.with_name('pages.json.gz').read_bytes()).hexdigest())
        with ZipFile(BytesIO(original)) as parent:
            original_styles = ET.fromstring(parent.read('word/styles.xml'))
            self.assertEqual(original_styles.get('{' + MC + '}Ignorable'), 'w14')
            self.assertNotIn('w14', original_styles.nsmap)
            for name, data in probes():
                with self.subTest(name=name), ZipFile(BytesIO(data)) as package:
                    expected = {'word/styles.xml'}
                    if name.startswith('word-windows-courier-line-'):
                        expected.add('word/document.xml')
                    if name.endswith('-compat15'):
                        expected.add('word/settings.xml')
                    self.assertEqual({p for p in parent.namelist()
                                      if package.read(p) != parent.read(p)}, expected)
                    styles = ET.fromstring(package.read('word/styles.xml'))
                    self.assertEqual(styles.nsmap['w14'], W14)
                    self.assertEqual(styles.get('{' + MC + '}Ignorable'), 'w14')
        self.assertEqual(source.read_bytes(), original)

    def test_word_version_executable_and_actual_courier_font_metrics_are_retained(self):
        records = {r['name']: r for r in read_json(
            ROOT / 'tests/word-oracle/windows/wrap-and-courier-observations.json')['fixtures']}
        for name, raw in probes():
            record = records[name]
            with self.subTest(name=name):
                self.assertEqual(record['coordinate_gate_promotion'], 'font-coordinates')
                self.assertEqual(record['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(record['word']['executable_sha256'],
                                 '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                capture = read_json(checked_file(ROOT, record['capture']))
                self.assertEqual(capture['word'], record['word'])
                self.assertEqual(capture['capture_id'], record['capture_id'])
                document, = capture['documents']
                self.assertEqual(document['open_mode'], 'normal-read-only')
                self.assertEqual(document['sha256'], hashlib.sha256(raw).hexdigest())
                self.assertEqual(document['compatibility_mode'],
                                 15 if name.endswith('-compat15') else 14)
                fonts = [p for p in record['original_word_font_programs']
                         if 'Courier New' in p['family']]
                self.assertTrue(fonts)
                for font in fonts:
                    self.assertEqual(font['version'], ['Version 6.95'])
                    self.assertEqual(font['upem'], 2048)
                    self.assertEqual(font['hhea'], [1705, -615, 0])
                    self.assertEqual(font['windows_box'], [1705, 615])


if __name__ == '__main__':
    unittest.main()
