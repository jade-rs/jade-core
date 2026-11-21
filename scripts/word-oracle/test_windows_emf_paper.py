"""Original custom-paper EMFs and precise reference-device corruption cases."""
import hashlib
import json
import struct
import unittest

import check_windows_paper_geometry as paper
from extract_windows_emf import emf_glyphs


class WordPaperMetafileTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.documents = []
        for case, paths in paper.verify():
            capture = paper.read_json(paths['capture'])
            document, = [r for r in capture['documents'] if r['name'] == case['name']]
            page, = document['metafiles']
            data = (paths['capture'].parent / page['file']).read_bytes()
            assert hashlib.sha256(data).hexdigest() == page['sha256']
            canvas, offset = None, 0
            while offset < len(data):
                kind, size = struct.unpack_from('<II', data, offset)
                if kind == 9:
                    canvas = struct.unpack_from('<2i', data, offset + 8)
                if kind == 84:
                    scale = struct.unpack_from('<ff', data, offset + 28)
                    break
                offset += size
            frame = struct.unpack_from('<2i', data, 32)
            delta = [scale[i] * canvas[i] - frame[i] for i in (0, 1)]
            cls.documents.append((case['name'], data, page, offset, delta))

    def decode(self, data, page):
        return emf_glyphs(data, page['page'], page['width_pt'], page['height_pt'])

    def rounded_record(self):
        return next(r for r in self.documents if max(abs(v) for v in r[4]) > 2)

    def test_original_seventeen_decode_and_thirteen_legacy_origins_stay_exact(self):
        legacy = {}
        for name, data, page, _, delta in self.documents:
            actual = self.decode(data, page)
            self.assertTrue(actual, name)
            if max(abs(v) for v in delta) <= 2:
                legacy[name] = actual
        self.assertEqual(len(legacy), 13)
        # Original Word bytes decoded by the unchanged merged-64c extractor.
        # This digest contains Word origins, never Jade's rendered positions.
        digest = hashlib.sha256(json.dumps(legacy, sort_keys=True, separators=(',', ':'),
                                           ensure_ascii=False).encode()).hexdigest()
        self.assertEqual(digest, '91bfe0d633b24734dea2ca2d9b813548e717af67460e59d0f6ff8c19b37f0be6')

    def test_rounded_metadata_rejects_a_forged_scale_or_reference_device(self):
        _, data, page, offset, delta = self.rounded_record()
        axis = max((0, 1), key=lambda i: abs(delta[i]))
        bad_scale = bytearray(data)
        field = offset + 28 + axis * 4
        bits, = struct.unpack_from('<I', data, field)
        struct.pack_into('<I', bad_scale, field, bits + 16)
        bad_device = bytearray(data)
        field = 72 + axis * 4
        pixels, = struct.unpack_from('<i', data, field)
        struct.pack_into('<i', bad_device, field, pixels + 1)
        for corrupted in (bad_scale, bad_device):
            with self.assertRaisesRegex(ValueError, 'Text scale does not describe'):
                self.decode(bytes(corrupted), page)

    def test_rounded_metadata_rejects_a_frame_outside_integer_rounding(self):
        _, data, page, _, delta = self.rounded_record()
        axis = max((0, 1), key=lambda i: abs(delta[i]))
        corrupted = bytearray(data)
        field = 32 + axis * 4
        edge, = struct.unpack_from('<i', data, field)
        struct.pack_into('<i', corrupted, field, edge - 16 if delta[axis] > 0 else edge + 16)
        with self.assertRaisesRegex(ValueError, 'Text scale does not describe'):
            self.decode(bytes(corrupted), page)


if __name__ == '__main__':
    unittest.main()
