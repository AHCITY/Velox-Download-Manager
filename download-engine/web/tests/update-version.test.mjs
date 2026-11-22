// Phase 6 — updater version-guard tests (node:test, no DOM needed).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseVersion, isNewerVersion } from '../assets/update-version.js';

test('parseVersion handles v-prefix, whitespace, suffixes and junk', () => {
  assert.deepEqual(parseVersion('0.4.0'), [0, 4, 0]);
  assert.deepEqual(parseVersion('v0.10.2'), [0, 10, 2]);
  assert.deepEqual(parseVersion(' 1.2.3-beta '), [1, 2, 3]);
  assert.equal(parseVersion('garbage'), null);
  assert.equal(parseVersion(''), null);
  assert.equal(parseVersion(null), null);
});

test('strictly newer is detected', () => {
  assert.equal(isNewerVersion('0.4.1', '0.4.0'), true);
  assert.equal(isNewerVersion('0.5.0', '0.4.9'), true);
  assert.equal(isNewerVersion('1.0.0', '0.9.9'), true);
  assert.equal(isNewerVersion('v0.4.1', '0.4.0'), true);
});

test('equal or older versions NEVER announce (the false-update bug)', () => {
  assert.equal(isNewerVersion('0.4.0', '0.4.0'), false);
  assert.equal(isNewerVersion('0.3.9', '0.4.0'), false);
  assert.equal(isNewerVersion('0.4.0', 'v0.4.0'), false);
  assert.equal(isNewerVersion('', '0.4.0'), false);
  assert.equal(isNewerVersion('latest', '0.4.0'), false);
  assert.equal(isNewerVersion(null, '0.4.0'), false);
  // unparseable installed version → honest no-announce
  assert.equal(isNewerVersion('0.5.0', 'dev'), false);
});

test('numeric (not lexicographic) comparison across digit widths', () => {
  assert.equal(isNewerVersion('0.10.0', '0.9.0'), true, '10 > 9 numerically');
  assert.equal(isNewerVersion('0.9.0', '0.10.0'), false);
});
