// Activity-first ordering (P4.3): active downloads on top, newest activity
// first; the user's sort governs the rest. Pure module — no DOM, no engine.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { orderRows, becameActive } from '../assets/ordering.js';

const ACTIVE = ['downloading', 'retrying', 'resolving', 'merging'];

const row = (id, state, extra = {}) => ({ id, state, final_name: `f${id}`, created_at: '', total_size: 100, received: 0, ...extra });

test('active rows sort above non-active rows regardless of user sort', () => {
  const rows = [
    row(1, 'completed', { created_at: 'b' }),
    row(2, 'downloading'),
    row(3, 'paused', { created_at: 'z' }),
    row(4, 'failed'),
  ];
  const out = orderRows(rows, { activeStates: ACTIVE, lastActive: new Map(), sortKey: 'created_at', sortDir: 'desc' });
  assert.deepEqual(out.map((r) => r.id), [2, 3, 1, 4]);
});

test('most recently started activity is on top (resume jumps back up)', () => {
  const lastActive = new Map([[10, 500], [11, 900]]);
  const rows = [row(10, 'downloading'), row(11, 'downloading'), row(12, 'completed')];
  const out = orderRows(rows, { activeStates: ACTIVE, lastActive, sortKey: 'created_at', sortDir: 'desc' });
  assert.equal(out[0].id, 11, 'resumed later → top');
  assert.equal(out[1].id, 10);
});

test('resolving and merging count as active work; queued does not', () => {
  const rows = [row(1, 'queued'), row(2, 'resolving'), row(3, 'merging'), row(4, 'downloading'), row(5, 'created')];
  const out = orderRows(rows, { activeStates: ACTIVE, lastActive: new Map(), sortKey: 'created_at', sortDir: 'desc' });
  assert.deepEqual(out.map((r) => r.state), ['downloading', 'merging', 'resolving', 'created', 'queued']);
});

test('rows never seen active fall back to id order inside the active group', () => {
  const rows = [row(3, 'downloading'), row(5, 'downloading'), row(4, 'downloading')];
  const out = orderRows(rows, { activeStates: ACTIVE, lastActive: new Map(), sortKey: 'created_at', sortDir: 'desc' });
  assert.deepEqual(out.map((r) => r.id), [5, 4, 3]);
});

test('user sort still governs the non-active group (name asc)', () => {
  const rows = [
    row(1, 'completed', { final_name: 'zzz' }),
    row(2, 'paused', { final_name: 'aaa' }),
    row(3, 'failed', { final_name: 'mmm' }),
    row(9, 'downloading'),
  ];
  const out = orderRows(rows, { activeStates: ACTIVE, lastActive: new Map(), sortKey: 'final_name', sortDir: 'asc' });
  assert.deepEqual(out.map((r) => r.id), [9, 2, 3, 1]);
});

test('the input array is never mutated', () => {
  const rows = [row(2, 'completed'), row(1, 'downloading')];
  const before = rows.map((r) => r.id);
  orderRows(rows, { activeStates: ACTIVE, lastActive: new Map(), sortKey: 'created_at', sortDir: 'desc' });
  assert.deepEqual(rows.map((r) => r.id), before);
});

test('becameActive: entering active from any non-active state stamps; staying does not', () => {
  assert.equal(becameActive('paused', 'downloading', ACTIVE), true);
  assert.equal(becameActive(undefined, 'downloading', ACTIVE), true, 'fresh active row');
  assert.equal(becameActive('queued', 'created', ACTIVE), false);
  assert.equal(becameActive('downloading', 'downloading', ACTIVE), false, 'no re-stamp churn on progress');
  assert.equal(becameActive('downloading', 'completed', ACTIVE), false);
  assert.equal(becameActive('resolving', 'merging', ACTIVE), false, 'active→active no re-stamp');
});
