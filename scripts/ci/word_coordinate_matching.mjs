// Reassign a matched candidate if another valid pairing frees it for this
// entry. Greedy matching can reject a complete pairing within tolerance.
function unmatchedCoordinates(expected, actual, within) {
  const owners = new Map();

  function assign(index, visited) {
    for (const [candidate, position] of actual.entries()) {
      if (owners.has(candidate) || !within(expected[index], position)) continue;
      owners.set(candidate, index);
      return true;
    }

    for (const [candidate, position] of actual.entries()) {
      if (visited.has(candidate) || !within(expected[index], position)) continue;
      visited.add(candidate);
      if (assign(owners.get(candidate), visited)) {
        owners.set(candidate, index);
        return true;
      }
    }

    return false;
  }

  return {
    missing: expected.filter((_, index) => !assign(index, new Set())),
    extra: actual.filter((_, index) => !owners.has(index)),
  };
}

export function matchImageBounds(expected, actual, within) {
  return expected.length === actual.length &&
    unmatchedCoordinates(expected, actual, within).missing.length === 0;
}

export function compareGlyphs(expected, actual, tolerance, toleranceX = tolerance) {
  const glyphs = new Map();
  for (const [side, positions] of [['expected', expected], ['actual', actual]]) {
    for (const glyph of positions) {
      const [text] = glyph;
      if (!glyphs.has(text)) glyphs.set(text, { expected: [], actual: [] });
      glyphs.get(text)[side].push(glyph);
    }
  }

  const result = { missing: [], extra: [] };
  const within = ([, x, y], [, ax, ay]) =>
    Math.abs(ax - x) <= toleranceX && Math.abs(ay - y) <= tolerance;
  for (const group of glyphs.values()) {
    const { missing, extra } = unmatchedCoordinates(group.expected, group.actual, within);
    for (const glyph of missing) result.missing.push(glyph);
    for (const glyph of extra) result.extra.push(glyph);
  }
  return result;
}
