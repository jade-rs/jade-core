"""Small independent MS-EMF streams exercise extraction and rejection paths."""
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import unittest

from extract_windows_emf import emf_glyphs, extract, verify_visible_reference


def record(kind, body):
    return struct.pack('<II', kind, len(body) + 8) + body


def metafile(origin=600, mode=1, flags=4, rounded_printer=False):
    # A US Letter whole-page canvas, 600dpi, baseline at 84pt.
    text = 'A日'.encode('utf-16-le')
    text_record = bytearray(88)
    struct.pack_into('<II', text_record, 0, 84, 88)
    struct.pack_into('<Iff', text_record, 24, mode,
                     (21600 if rounded_printer else 21590) / 5100,
                     (27900 if rounded_printer else 27940) / 6600)
    struct.pack_into('<iiIII', text_record, 36, origin, 700, 2, 76, flags)
    struct.pack_into('<I', text_record, 72, 80)
    text_record[76:80] = text
    struct.pack_into('<ii', text_record, 80, 50, 100)
    records = [record(9, struct.pack('<ii', 5100, 6600)),
               record(22, struct.pack('<I', 24)), bytes(text_record),
               record(14, struct.pack('<III', 0, 0, 20))]
    header = bytearray(108)
    struct.pack_into('<II', header, 0, 1, 108)
    struct.pack_into('<iiii', header, 24, 0, 0,
                     21600 if rounded_printer else 21590,
                     27900 if rounded_printer else 27940)
    struct.pack_into('<IIII', header, 40, 0x464d4520, 0x10000,
                     108 + sum(map(len, records)), 1 + len(records))
    return bytes(header) + b''.join(records)


class DrawingOriginTests(unittest.TestCase):
    def test_raw_origin_and_advances_convert_without_alignment(self):
        self.assertEqual(emf_glyphs(metafile(), 1, 612, 792),
                         [[1, 'A', 72, 84], [1, '日', 78, 84]])

    def test_shifted_origin_is_not_normalized_back(self):
        actual = emf_glyphs(metafile(origin=636), 1, 612, 792)
        self.assertAlmostEqual(actual[0][2], 76.32)
        self.assertGreater(abs(actual[0][2] - 72), 0.375)

    def test_word_canvas_is_independent_of_rounded_printer_millimetres(self):
        self.assertEqual(emf_glyphs(metafile(rounded_printer=True), 1, 612, 792),
                         emf_glyphs(metafile(), 1, 612, 792))

    def test_truncated_and_appended_streams_are_rejected(self):
        for data in [b'', metafile()[:-1], metafile() + b'junk']:
            with self.subTest(size=len(data)), self.assertRaises(ValueError):
                emf_glyphs(data, 1, 612, 792)

    def test_glyph_indices_and_other_graphics_modes_are_not_guessed(self):
        for data in [metafile(mode=2), metafile(flags=0x10), metafile(flags=0x80)]:
            with self.assertRaises(ValueError):
                emf_glyphs(data, 1, 612, 792)

    def test_wrong_paper_and_string_or_advance_bounds_are_rejected(self):
        with self.assertRaises(ValueError):
            emf_glyphs(metafile(), 1, 300, 792)
        # Header + canvas + text alignment place the text record at byte 136.
        for field in [48, 72]:
            data = bytearray(metafile())
            struct.pack_into('<I', data, 136 + field, 0xffffffff)
            with self.assertRaises(ValueError):
                emf_glyphs(data, 1, 612, 792)

    def test_rotated_font_is_not_interpreted_as_horizontal_text(self):
        font = bytearray(360)
        struct.pack_into('<I', font, 0, 1)
        struct.pack_into('<i', font, 12, 900)
        data = bytearray(metafile())
        inserted = record(82, bytes(font)) + record(37, struct.pack('<I', 1))
        data[136:136] = inserted
        struct.pack_into('<I', data, 48, len(data))
        struct.pack_into('<I', data, 52, 7)
        with self.assertRaisesRegex(ValueError, 'mode, alignment or transform'):
            emf_glyphs(data, 1, 612, 792)

    def test_capture_hashes_page_order_and_full_word_build_are_required(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            data = metafile()
            (root / 'page.emf').write_bytes(data)
            pdf = b'independent Word PDF companion'
            (root / 'probe.pdf').write_bytes(pdf)
            page = dict(page=1, file='page.emf', sha256=hashlib.sha256(data).hexdigest(),
                        width_pt=612, height_pt=792)
            capture = dict(schema=2, status='complete', platform='windows',
                           word=dict(product_version='16.0.19127.20860'),
                           capture_id='20261103T000000000Z-1234abcd', captured_at_utc='capture-time',
                           documents=[dict(name='probe', sha256='source-hash', pdf='probe.pdf',
                                           pdf_sha256=hashlib.sha256(pdf).hexdigest(), pages=1, metafiles=[page])])
            manifest = root / 'capture.json'

            def write():
                manifest.write_text(json.dumps(capture), encoding='utf-8')

            write()
            self.assertEqual(len(extract(manifest)['fixtures'][0]['glyphs']), 2)
            page['page'] = 2
            write()
            with self.assertRaisesRegex(ValueError, 'pages'):
                extract(manifest)
            page['page'] = 1
            capture['word']['product_version'] = '16.0'
            write()
            with self.assertRaisesRegex(ValueError, 'version'):
                extract(manifest)
            capture['word']['product_version'] = '16.0.19127.20860'
            write()
            (root / 'page.emf').write_bytes(data + b'tampered')
            with self.assertRaisesRegex(ValueError, 'changed'):
                extract(manifest)


class VisiblePdfProofTests(unittest.TestCase):
    """Synthetic clipped text proves the extra visible plane cannot be skipped."""
    def setUp(self):
        import pymupdf
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root/'inputs').mkdir()
        source = b'synthetic DOCX integrity input'
        (self.root/'inputs/probe.docx').write_bytes(source)
        doc = pymupdf.open()
        page = doc.new_page(width=100, height=100)
        page.insert_text((20,40), 'A', fontsize=12)
        page.insert_text((20,90), 'B', fontsize=12)
        stream = page.get_contents()[-1]
        doc.update_stream(stream, b'q\n0 40 100 60 re W n\n'+doc.xref_stream(stream)+b'\nQ\n')
        pdf = doc.tobytes()
        (self.root/'probe.pdf').write_bytes(pdf)
        fixture = dict(name='probe', docx_sha256=hashlib.sha256(source).hexdigest(),
                       pdf_sha256=hashlib.sha256(pdf).hexdigest(), pdf_file='probe.pdf',
                       page_count=1, glyphs=[[1,'A',20.,40.],[1,'B',20.,90.]])
        self.full = dict(schema=2, platform='windows', word_product_version='16.0.19127.20860',
                         word=dict(product_version='16.0.19127.20860'), capture_id='synthetic',
                         captured_at_utc='2026-11-01T01:02:03Z', tolerance_pt=.375,
                         table_glyph_projection='unclipped', fixtures=[fixture])
        self.visible = dict(self.full, table_glyph_projection='visible',
                            pdf_text_flags=pymupdf.TEXTFLAGS_RAWDICT,
                            extractor=f'PyMuPDF {pymupdf.VersionBind}',
                            fixtures=[dict(fixture, glyphs=[[1,'A',20.,40.]])])
        self.entry = dict(visible_reference='strict-visible.json', visible_glyph_count=1)
        self.write()

    def write(self):
        (self.root/'strict-visible.json').write_bytes((json.dumps(self.visible)+'\n').encode())

    def verify(self):
        return verify_visible_reference(self.root, self.entry, self.full, self.full)

    def test_original_visible_pdf_plane_is_independent_of_hidden_text(self):
        self.assertEqual(self.verify(), 1)
        self.assertEqual(len(self.full['fixtures'][0]['glyphs']), 2)

    def test_shifted_missing_reordered_or_repaginated_visible_text_is_rejected(self):
        variants = [[[1,'A',21.,40.]], [], [[1,'B',20.,40.]], [[2,'A',20.,40.]]]
        for glyphs in variants:
            with self.subTest(glyphs=glyphs):
                self.visible['fixtures'][0]['glyphs'] = glyphs
                self.write()
                with self.assertRaisesRegex(ValueError, 'Visible origins'):
                    self.verify()

    def test_missing_visible_reference_is_rejected(self):
        self.entry.pop('visible_reference')
        with self.assertRaisesRegex(ValueError, 'require a visible reference'):
            self.verify()

    def test_clipping_flags_and_word_build_are_not_interchangeable(self):
        self.visible['pdf_text_flags'] &= ~64
        self.write()
        with self.assertRaisesRegex(ValueError, 'flags or decoder'):
            self.verify()
        self.visible['pdf_text_flags'] |= 64
        self.visible['word_product_version'] = '16.0.20000.10000'
        self.write()
        with self.assertRaisesRegex(ValueError, 'provenance'):
            self.verify()

    def test_source_docx_tampering_is_rejected(self):
        (self.root/'inputs/probe.docx').write_bytes(b'tampered')
        with self.assertRaisesRegex(ValueError, 'source DOCX'):
            self.verify()

    def test_visible_coverage_floor_cannot_be_lowered(self):
        self.entry['visible_glyph_count'] = 0
        with self.assertRaisesRegex(ValueError, 'coverage'):
            self.verify()


if __name__ == '__main__':
    unittest.main()
