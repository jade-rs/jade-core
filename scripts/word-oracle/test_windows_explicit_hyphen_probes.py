"""Protect distinct OOXML marker forms and versioned original Word evidence."""
from io import BytesIO
import itertools
import json
import unittest
from zipfile import ZipFile

from lxml import etree as ET
from check_platform_coordinates import checked_file, compare
import generate_windows_explicit_hyphen_probes as producer

ROOT = producer.ROOT
read = lambda p: json.loads(p.read_text(encoding='utf-8-sig'))


class ExplicitHyphenProbes(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.metadata = read(ROOT / 'tests/word-oracle/windows/explicit-hyphen-observations.json')
        cls.rows = cls.metadata['fixtures']

    def test_all60_original_inputs_reproduce_byte_for_byte(self):
        generated = {name: (data, facts) for name, data, facts in producer.generate()}
        self.assertEqual(len(generated), 60)
        self.assertEqual(set(generated), {r['name'] for r in self.rows})
        combinations = set()
        for row in self.rows:
            with self.subTest(case=row['name']):
                data, facts = generated[row['name']]
                self.assertEqual(data, checked_file(ROOT, row['source']).read_bytes())
                self.assertEqual(facts, row['input_specification'])
                combinations.add((facts['marker_kind'], facts['available_width_pt'], facts['compatibility_mode']))
        self.assertEqual(combinations, set(itertools.product(producer.KINDS, producer.WIDTHS, producer.MODES)))
        with self.assertRaisesRegex(ValueError, 'Original Mac hyphen source changed'):
            list(producer.generate(b'changed original'))

    def test_literal_and_element_forms_stay_distinct_and_other_parts_are_preserved(self):
        q = producer.q
        with ZipFile(ROOT / producer.PARENT) as parent:
            parent_document = ET.fromstring(parent.read('word/document.xml'))
            original_run = parent_document.find(q('body')).findall(q('p'))[1].find(q('r'))
            for row in self.rows:
                facts = row['input_specification']
                with self.subTest(case=row['name']), ZipFile(checked_file(ROOT, row['source'])) as package:
                    self.assertEqual(package.namelist(), parent.namelist())
                    for part in parent.namelist():
                        if part not in ('word/document.xml', 'word/settings.xml'):
                            self.assertEqual(package.read(part), parent.read(part), part)
                    document = ET.fromstring(package.read('word/document.xml'))
                    body = document.find(q('body'))
                    self.assertEqual(len(body.findall(q('p'))), 1)
                    self.assertEqual(ET.tostring(body.find(q('sectPr'))), ET.tostring(parent_document.find(q('body')).find(q('sectPr'))))
                    run = body.find(q('p')).find(q('r'))
                    self.assertEqual([ET.tostring(p) for p in run.findall(q('rPr'))],
                                     [ET.tostring(p) for p in original_run.findall(q('rPr'))])
                    kind = facts['marker_kind']
                    self.assertEqual(len(run.findall(q('softHyphen'))), int(kind == 'soft-element'))
                    self.assertEqual(len(run.findall(q('noBreakHyphen'))), int(kind == 'nonbreak-element'))
                    marker = '\u00ad' if kind == 'soft-literal' else '-' if kind == 'ascii' else ''
                    self.assertEqual(''.join(t.text or '' for t in body.find(q('p')).iter(q('t'))),
                                     'electro' + marker + 'magnetic')

    def test_actual_word_executable_font_and_mac_lineage_remain_identifiable(self):
        manifest = read(checked_file(ROOT, self.metadata['mac_parent_manifest']))
        self.assertEqual(manifest['reference'], self.metadata['mac_parent_reference'])
        self.assertEqual(manifest['reference']['platform'], 'mac')
        self.assertEqual(manifest['reference']['version'], '16.114.4')
        self.assertEqual(manifest['sourceSha256'], producer.PARENT_SHA256)
        for row in self.rows:
            with self.subTest(case=row['name']):
                capture = read(checked_file(ROOT, row['capture']))
                document, = capture['documents']
                self.assertEqual(capture['word'], row['word'])
                self.assertEqual(row['word']['product_version'], '16.0.19127.20936')
                self.assertEqual(row['word']['executable_sha256'], '7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312')
                self.assertEqual(document['sha256'], row['source']['sha256'])
                self.assertEqual(document['compatibility_mode'], row['input_specification']['compatibility_mode'])
                self.assertEqual(document['open_mode'], 'normal-read-only')
                self.assertEqual(row['coordinate_gate_promotion'], 'pending')
                programs = [p for p in row['original_word_font_programs'] if 'Arial' in p['family']]
                self.assertTrue(programs)
                for program in programs:
                    self.assertEqual(program['version'], ['Version 7.07'])
                    self.assertRegex(program['program_sha256'], r'^[a-f0-9]{64}$')
                with ZipFile(checked_file(ROOT, row['archive'])) as archive:
                    self.assertEqual(archive.read('capture.json'), checked_file(ROOT, row['capture']).read_bytes())
                    self.assertEqual(archive.read('inputs/' + row['name'] + '.docx'), checked_file(ROOT, row['source']).read_bytes())

    def test_word_painting_requires_distinct_literal_and_optional_hyphen_semantics(self):
        import pymupdf

        def painted(row):
            manifest = checked_file(ROOT, row['capture'])
            document, = read(manifest)['documents']
            with pymupdf.open(manifest.parent / document['pdf']) as pdf:
                glyphs = [[page_index, chr(char), *origin]
                          for page_index, page in enumerate(pdf, 1)
                          for span in page.get_texttrace() if span['type'] != 3
                          for char, glyph_id, origin, box in span['chars']
                          if not chr(char).isspace()]
                return dict(page_count=len(pdf), glyphs=glyphs)

        models = {(r['input_specification']['marker_kind'], r['input_specification']['available_width_pt'],
                   r['input_specification']['compatibility_mode']): painted(r) for r in self.rows}
        for width, mode in itertools.product(producer.WIDTHS, producer.MODES):
            with self.subTest(width=width, mode=mode):
                literal = models['soft-literal', width, mode]
                nonbreaking = models['nonbreak-element', width, mode]
                optional = models['soft-element', width, mode]
                # These are independent original Word exports, not native output
                # or a logical dump with the source controls filtered away.
                self.assertEqual(compare(literal, nonbreaking, 'literal-vs-noBreakHyphen'), 16)
                if width in (35, 65, 110):
                    with self.assertRaisesRegex(ValueError, 'axis mismatch|missing/extra glyphs'):
                        compare(literal, optional, 'literal-vs-softHyphen')


if __name__ == '__main__':
    unittest.main()
