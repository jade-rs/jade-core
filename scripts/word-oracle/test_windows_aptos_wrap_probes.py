"""Retain source reproduction and Word provenance for pending Aptos controls."""
import hashlib
from io import BytesIO
import unittest
from zipfile import ZipFile

from check_platform_coordinates import ROOT, checked_file, read_json
from generate_windows_aptos_wrap_kerning_probes import PARENT, probes


class AptosWrapProbes(unittest.TestCase):
    def test_all_32_generated_inputs_match_the_original_word_captures(self):
        parent = (ROOT / PARENT).read_bytes()
        self.assertEqual(hashlib.sha256(parent).hexdigest(),
                         'ce26341fd72f4480edc033ee2093d803ad8e1527723255ab0980c760b7e1dff0')
        records = {r['name']: r for r in read_json(
            ROOT / 'tests/word-oracle/windows/wrap-and-courier-observations.json')['fixtures']
                   if r['group'] == 'long-kerning'}
        generated = dict(probes())
        self.assertEqual(len(generated), 32)
        self.assertEqual(set(generated), set(records))
        with ZipFile(BytesIO(parent)) as original:
            for name, data in generated.items():
                with self.subTest(name=name), ZipFile(BytesIO(data)) as package:
                    self.assertEqual(checked_file(ROOT, records[name]['source']).read_bytes(), data)
                    expected = {'word/document.xml'}
                    if name.endswith('-compat15'):
                        expected.add('word/settings.xml')
                    self.assertEqual({part for part in original.namelist()
                                      if original.read(part) != package.read(part)}, expected)
        self.assertEqual((ROOT / PARENT).read_bytes(), parent)

    def test_original_word_and_embedded_font_versions_cannot_be_relabelled(self):
        records = {r['name']: r for r in read_json(
            ROOT / 'tests/word-oracle/windows/wrap-and-courier-observations.json')['fixtures']}
        for name, data in probes():
            row = records[name]
            with self.subTest(name=name):
                self.assertEqual(row['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(row['word']['executable_sha256'],
                                 '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                capture = read_json(checked_file(ROOT, row['capture']))
                self.assertEqual(capture['word'], row['word'])
                document, = capture['documents']
                self.assertEqual(document['open_mode'], 'normal-read-only')
                self.assertEqual(document['sha256'], hashlib.sha256(data).hexdigest())
                fonts = [p for p in row['original_word_font_programs']
                         if set(p['family']) & {'Aptos', 'Aptos Display'}]
                self.assertTrue(fonts)
                self.assertTrue(all(p['version'] == ['Version 2.01;O365'] for p in fonts))


if __name__ == '__main__':
    unittest.main()
