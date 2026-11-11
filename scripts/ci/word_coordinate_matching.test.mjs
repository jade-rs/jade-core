import assert from 'node:assert/strict';
import test from 'node:test';
import { compareGlyphs, matchImageBounds } from './word_coordinate_matching.mjs';

test('repeated glyph origins are matched one-to-one even when ranges overlap', () => {
  const expected = [['A', 10, 20], ['A', 11, 20]];
  const actual = [['A', 10.5, 20], ['A', 9.5, 20]];
  assert.deepEqual(compareGlyphs(expected, actual, 0.65), { missing: [], extra: [] });
});

test('glyphs beyond an axis tolerance or with different text remain unmatched', () => {
  const expected = [['A', 10, 20], ['B', 11, 20], ['C', 12, 20]];
  const actual = [['A', 10.5, 20], ['B', 11, 20.7], ['D', 12, 20]];
  assert.deepEqual(compareGlyphs(expected, actual, 0.65, 0.4), {
    missing: expected,
    extra: actual,
  });
});

test('overlapping image bounds are matched one-to-one regardless of paint order', () => {
  const expected = [[10, 20, 30, 40], [11, 20, 31, 40]];
  const actual = [[10.5, 20, 30.5, 40], [9.5, 20, 29.5, 40]];
  const within = (a, b) => a.every((coordinate, i) => Math.abs(coordinate - b[i]) <= 0.65);
  assert.equal(matchImageBounds(expected, actual, within), true);
  assert.equal(matchImageBounds(expected, actual.slice(0, 1), within), false);
  assert.equal(matchImageBounds(expected, [actual[1], actual[1]], within), false);
});
