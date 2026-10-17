import unittest
import numpy as np
from raster_parity import compare, reader_image_reference


class RasterParityTests(unittest.TestCase):
    def page(self):
        page = np.full((128, 128, 3), 255, dtype=np.uint8)
        page[20:80, 20:80] = 0
        return page

    def test_identical_rasters_match(self):
        page = self.page()
        self.assertTrue(compare(page, page)["matches"])

    def test_wrong_page_dimensions_are_rejected_even_when_blank(self):
        source = np.full((128, 128, 3), 255, dtype=np.uint8)
        actual = np.full((140, 128, 3), 255, dtype=np.uint8)
        self.assertFalse(compare(source, actual)["matches"])

    def test_rounded_extra_blank_edge_pixel_is_allowed(self):
        source = self.page()
        actual = np.pad(source, ((0, 1), (0, 1), (0, 0)), constant_values=255)
        self.assertTrue(compare(source, actual)["matches"])

    def test_one_pixel_reader_sampling_offset_is_allowed(self):
        source = self.page()
        actual = np.roll(source, 1, axis=1)
        self.assertTrue(compare(source, actual)["matches"])

    def test_fractional_reader_sampling_at_a_tile_boundary_is_allowed(self):
        source = np.full((128, 128, 3), 255, dtype=np.uint8)
        source[40:80, 63] = 0
        shifted = np.roll(source, 1, axis=1)
        actual = np.rint(source.astype(float) * 0.6 + shifted.astype(float) * 0.4).astype(np.uint8)
        self.assertTrue(compare(source, actual)["matches"])

    def test_color_change_is_rejected_with_identical_ink_geometry(self):
        source = self.page()
        source[20:80, 20:80] = [255, 0, 0]
        actual = source.copy()
        actual[20:80, 20:80] = [0, 0, 255]
        self.assertFalse(compare(source, actual)["matches"])

    def test_density_change_is_rejected_with_identical_ink_geometry(self):
        source = self.page()
        actual = source.copy()
        actual[20:80, 20:80] = 100
        self.assertFalse(compare(source, actual)["matches"])

    def test_doubled_one_pixel_rule_is_rejected(self):
        for y in range(48, 64):
            with self.subTest(y=y):
                source = np.full((128, 128, 3), 255, dtype=np.uint8)
                source[y, 20:100] = 0
                actual = source.copy()
                actual[y + 1, 20:100] = 0
                self.assertFalse(compare(source, actual)["matches"])

    def test_small_missing_object_is_not_hidden_by_page_ink_budget(self):
        source = self.page()
        source[100, 100:102] = 0
        actual = source.copy()
        actual[100, 100:102] = 255
        self.assertFalse(compare(source, actual)["matches"])

    def test_reader_integer_viewport_sampling_is_predicted_from_page_geometry(self):
        source = np.full((81, 97, 3), 255, dtype=np.uint8)
        source[8:73, 60] = [30, 90, 180]
        actual = reader_image_reference(source, (82, 98), page_width=97.4,
                                        page_height=81.4, scale=1, reader="pdfium")
        self.assertEqual(actual.shape, (82, 98, 3))
        self.assertTrue(np.all(np.abs(actual[20, 60].astype(int) - [168, 191, 226]) <= 1))
        self.assertTrue(np.all(np.abs(actual[20, 61].astype(int) - [115, 152, 208]) <= 1))

    def test_reader_sampling_does_not_hide_a_recolored_thin_rule(self):
        source = np.full((81, 97, 3), 255, dtype=np.uint8)
        source[8:73, 60] = [30, 90, 180]
        expected = reader_image_reference(source, (82, 98), page_width=97.4,
                                         page_height=81.4, scale=1, reader="pdfium")
        changed = source.copy()
        changed[8:73, 60] = [180, 90, 30]
        actual = reader_image_reference(changed, (82, 98), page_width=97.4,
                                       page_height=81.4, scale=1, reader="pdfium")
        self.assertFalse(compare(expected, actual)["matches"])

    def test_poppler_inclusive_image_bounds_at_integer_page_dimensions(self):
        source = np.full((81, 97, 3), 255, dtype=np.uint8)
        source[8:73, 60] = [30, 90, 180]
        pdfium = reader_image_reference(source, (81, 97), page_width=97,
                                       page_height=81, scale=1, reader="pdfium")
        np.testing.assert_array_equal(pdfium, source)
        poppler = reader_image_reference(source, (81, 97), page_width=97,
                                        page_height=81, scale=1, reader="poppler")
        self.assertTrue(np.all(np.abs(poppler[20, 60].astype(int) - [167, 191, 225]) <= 2))
        self.assertTrue(np.all(np.abs(poppler[20, 61].astype(int) - [114, 152, 208]) <= 2))

    def test_poppler_quantizes_both_image_interpolation_passes(self):
        source = np.full((5, 7, 3), 255, dtype=np.uint8)
        source[2, 3] = [30, 90, 180]
        source[3, 4] = 64
        sampled = reader_image_reference(source, (5, 7), page_width=7,
                                         page_height=5, scale=1, reader="poppler")
        # Captured from pdftoppm's default Splash renderer, including byte
        # truncation between its horizontal and vertical interpolation passes.
        self.assertEqual(sampled[2, 3].tolist(), [161, 185, 223])
        self.assertEqual(sampled[2, 4].tolist(), [179, 199, 229])
        self.assertEqual(sampled[3, 4].tolist(), [150, 165, 188])

    def test_poppler_top_origin_rounding_uses_the_pdf_transform(self):
        source = np.full((4490, 28, 3), 255, dtype=np.uint8)
        source[0] = [30, 90, 180]
        sampled = reader_image_reference(source, (4491, 28), page_width=7,
                                         page_height=1122.5333658854167, scale=4, reader="poppler")
        # The PDF's top-aligned image CTM sums to -9e-13 device pixels.
        # Splash floors that lower bound to -1, then clips its first row.
        self.assertEqual(sampled[0, 10].tolist(), [254, 254, 254])
        self.assertEqual(sampled[1, 10].tolist(), [255, 255, 255])

    def test_poppler_decimal_coordinates_can_leave_image_width_unscaled(self):
        source = np.full((81, 3175, 3), 255, dtype=np.uint8)
        source[:, 407] = [30, 90, 180]
        sampled = reader_image_reference(source, (81, 3175),
                                         page_width=793.7333170572916,
                                         page_height=81 / 4, scale=4, reader="poppler")
        # pdftoppm parses the PDF's 595.3125-point image width slightly below
        # that value, so its inclusive upper bound remains 3175 pixels.
        self.assertEqual(sampled[20, 407].tolist(), [30, 90, 180])
        self.assertEqual(sampled[20, 408].tolist(), [255, 255, 255])

    def test_pdfium_float_transform_can_move_the_image_top_by_a_row(self):
        source = np.full((2245, 56, 3), 255, dtype=np.uint8)
        source[555] = [30, 90, 180]
        sampled = reader_image_reference(source, (2246, 56), page_width=28,
                                         page_height=1122.6666666666667, scale=2, reader="pdfium")
        # Captured from PDFium's default renderer at 192 DPI. Its float CTM
        # places the top at -0.000244 pixels before the outer rectangle floors.
        self.assertTrue(np.all(np.abs(sampled[554, 10].astype(int) - [141, 171, 217]) <= 1))
        self.assertTrue(np.all(np.abs(sampled[555, 10].astype(int) - [143, 173, 217]) <= 1))


if __name__ == "__main__":
    unittest.main()
