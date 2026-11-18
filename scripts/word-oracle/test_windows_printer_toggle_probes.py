"""Preserve setting-only printer controls and actual Word output provenance."""
from io import BytesIO
from zipfile import ZipFile
import hashlib
import itertools
import unittest
from lxml import etree as E
from check_platform_coordinates import ROOT, checked_file, read_json, pdf_origins
from extract_windows_emf import extract
import generate_windows_printer_toggle_probes as producer


class PrinterMetricProbes(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.metadata = read_json(ROOT / 'tests/word-oracle/windows/printer-metric-observations.json')
        cls.rows = cls.metadata['fixtures']
        cls.parents = {r['source']['sha256']: r for r in read_json(
            ROOT / 'tests/word-oracle/windows/aptos-nominal-observations.json')['fixtures']}

    def test_all16_sources_are_reproducible_with_complete_toggle_coverage(self):
        generated = {name: (data, spec) for name, data, spec in producer.generate()}
        self.assertEqual(len(generated), 16)
        self.assertEqual(set(generated), {r['name'] for r in self.rows})
        coverage = set()
        for row in self.rows:
            data, spec = generated[row['name']]
            self.assertEqual(data, checked_file(ROOT, row['source']).read_bytes())
            self.assertEqual(spec, row['input_specification'])
            coverage.add((spec['style'], spec['character'], spec['compatibility_mode'], spec['use_printer_metrics']))
        self.assertEqual(coverage, {(style, char, mode, enabled)
            for (style, char), mode, enabled in itertools.product(
                [('bold', 'X'), ('bolditalic', 'X'), ('regular', '1'), ('regular', '-')],
                (14, 15), (False, True))})

    def test_only_the_declared_printer_metric_flag_changes(self):
        q = producer.q
        for row in self.rows:
            spec = row['input_specification']
            with self.subTest(case=row['name']), ZipFile(checked_file(ROOT, spec['derived_parent'])) as before, \
                    ZipFile(checked_file(ROOT, row['source'])) as after:
                self.assertEqual(set(before.namelist()), set(after.namelist()))
                for part in before.namelist():
                    if part != 'word/settings.xml':
                        self.assertEqual(before.read(part), after.read(part))
                old, new = [E.fromstring(z.read('word/settings.xml')) for z in (before, after)]
                compat = new.find(q('compat'))
                flag, = compat.findall(q('usePrinterMetrics'))
                self.assertEqual(flag.get(q('val')), str(int(spec['use_printer_metrics'])))
                compat.remove(flag)
                self.assertEqual(E.tostring(old), E.tostring(new))

    def test_original_word_screen_and_print_are_independent_unchanged_parent_outputs(self):
        cache = {}
        for row in self.rows:
            path = checked_file(ROOT, row['capture']); capture = read_json(path)
            self.assertEqual(capture['word'], row['word'])
            self.assertEqual(capture['word']['product_version'], '16.0.19127.20936')
            self.assertEqual(capture['word']['executable_sha256'],
                '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
            self.assertEqual(capture['print_environment'], self.metadata['print_environment'])
            self.assertTrue(capture['print_environment']['active_printer'])
            parent = self.parents[row['input_specification']['derived_parent']['sha256']]
            parent_path = checked_file(ROOT, parent['capture']); parent_capture = read_json(parent_path)
            doc, = [d for d in capture['documents'] if d['name'] == row['name']]
            parent_doc, = [d for d in parent_capture['documents'] if d['name'] == parent['name']]
            self.assertEqual(doc['open_mode'], 'normal-read-only')
            with ZipFile(checked_file(ROOT, row['archive'])) as archive:
                self.assertEqual(archive.read('capture.json'), path.read_bytes())
                for filename, digest in [(doc['pdf'], doc['pdf_sha256'])] + [
                        (p['file'], p['sha256']) for p in doc['metafiles']]:
                    data = (path.parent / filename).read_bytes()
                    self.assertEqual(data, archive.read(filename))
                    self.assertEqual(hashlib.sha256(data).hexdigest(), digest)
            self.assertEqual(pdf_origins(path.parent / doc['pdf'], True),
                             pdf_origins(parent_path.parent / parent_doc['pdf'], True))
            for manifest in (path, parent_path):
                if manifest not in cache:
                    cache[manifest] = {r['name']: r for r in extract(manifest)['fixtures']}
            a, b = cache[path][row['name']], cache[parent_path][parent['name']]
            self.assertEqual(a['page_count'], b['page_count'])
            self.assertEqual(a['glyphs'], b['glyphs'])
            self.assertEqual(row['screen_gate_promotion'], 'aptos-nominal-screen')
            self.assertEqual(row['coordinate_gate_promotion'],
                             'aptos-nominal' if row['input_specification']['style'] == 'bold' else 'pending')


if __name__ == '__main__':
    unittest.main()
