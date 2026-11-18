"""Reproduce color-only spans and preserve independent original Word exports."""
import copy
from io import BytesIO
import unittest
from zipfile import ZipFile

from lxml import etree as E
from check_platform_coordinates import ROOT, checked_file, read_json
import check_windows_color_spans as gate
import generate_windows_color_span_probes as producer


class WindowsColorSpans(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.index = read_json(ROOT / gate.INDEX)
        cls.generated = {name: (data, spec) for name, data, spec in producer.generate()}
        cls.cases, cls.pending, cls.xps = gate.verify()

    def test_all52_sources_and_specifications_reproduce_exactly(self):
        self.assertEqual(len(self.generated), 52)
        self.assertEqual(set(self.generated), {r['name'] for r in self.index['fixtures']})
        for case in self.index['fixtures']:
            data, spec = self.generated[case['name']]
            self.assertEqual(data, checked_file(ROOT, case['source']).read_bytes())
            self.assertEqual(spec, case['input_specification'])
            if spec['interval'] == 0:
                self.assertEqual(data, checked_file(ROOT, spec['parent']).read_bytes())
        self.assertEqual(len(self.cases), 52)
        self.assertEqual(len(self.pending), 39)
        self.assertEqual(len(self.xps), 52)
        self.assertEqual(self.index['word']['product_version'], '16.0.19127.20936')
        self.assertEqual(self.index['mac_parent_reference']['version'], '16.114.3')

    def test_only_run_boundaries_and_color_change(self):
        q = producer.q
        for name, (data, spec) in self.generated.items():
            if spec['interval'] == 0:
                continue
            with self.subTest(case=name), ZipFile(checked_file(ROOT, spec['parent'])) as before, \
                    ZipFile(BytesIO(data)) as after:
                self.assertEqual(set(before.namelist()), set(after.namelist()))
                for part in before.namelist():
                    if part != 'word/document.xml':
                        self.assertEqual(before.read(part), after.read(part))
                roots = [E.fromstring(z.read('word/document.xml')) for z in (before, after)]
                for old, new in zip(roots[0].iter(q('p')), roots[1].iter(q('p')), strict=True):
                    original, = old.findall(q('r')); props = original.find(q('rPr'))
                    runs = new.findall(q('r'))
                    self.assertEqual(''.join(t.text or '' for t in old.iter(q('t'))),
                                     ''.join(t.text or '' for t in new.iter(q('t'))))
                    self.assertEqual(E.tostring(old.find(q('pPr'))), E.tostring(new.find(q('pPr'))))
                    for number, run in enumerate(runs):
                        properties = run.find(q('rPr')); color, = properties.findall(q('color'))
                        self.assertEqual(color.get(q('val')), '000001' if
                                         spec['color_mode'] == 'alternating-nearblack' and number % 2 else '000000')
                        properties.remove(color)
                        self.assertEqual(E.tostring(properties), E.tostring(props))

    def test_required_print_or_screen_cases_cannot_be_dropped(self):
        for field in ('coordinate_gate_promotion', 'xps_gate_promotion'):
            changed = copy.deepcopy(self.index)
            yes = next(r for r in changed['fixtures'] if r[field] != 'pending')
            no = next(r for r in changed['fixtures'] if r[field] == 'pending')
            yes[field], no[field] = no[field], yes[field]
            with self.assertRaisesRegex(ValueError, 'Required color-span print assertions changed'):
                gate.locked_promotions(changed)
        changed = copy.deepcopy(self.index)
        changed['fixtures'][0]['screen_gate_promotion'] = 'pending'
        with self.assertRaisesRegex(ValueError, 'screen assertion removed'):
            gate.locked_promotions(changed)

    def test_word_screen_identity_does_not_erase_real_pdf_segmentation_differences(self):
        controls = {case['input_specification']['style']: (drawing, full)
                    for case, _, _, drawing, _, full in self.cases
                    if case['input_specification']['interval'] == 0}
        biggest = 0.
        for case, _, _, drawing, _, full in self.cases:
            screen, physical = controls[case['input_specification']['style']]
            self.assertEqual(drawing['glyphs'], screen['glyphs'])
            self.assertEqual([g[:2] for g in full['glyphs']], [g[:2] for g in physical['glyphs']])
            biggest = max(biggest, max(abs(a[2] - b[2]) for a, b in zip(full['glyphs'], physical['glyphs'], strict=True)))
        self.assertGreater(biggest, .25)


if __name__ == '__main__':
    unittest.main()
