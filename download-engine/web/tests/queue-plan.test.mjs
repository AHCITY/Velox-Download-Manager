// Queue orchestrator decision logic (pure module — no DOM needed).
import test from 'node:test';
import assert from 'node:assert/strict';
import { planQueueStarts, setQueueHold } from '../assets/queue-plan.js';

const rows = (list) => new Map(list.map(([id, state]) => [id, { id, state }]));

test('starts waiting members up to the concurrency limit, never more', () => {
  const queues = [{ active: true, maxConcurrent: 2, itemIds: [1, 2, 3, 4] }];
  const map = rows([[1, 'downloading'], [2, 'queued'], [3, 'paused'], [4, 'created']]);
  // 1 running -> one free slot, filled by the first waiting member in order
  assert.deepEqual(planQueueStarts(queues, map), [2]);
});

test('a manually paused member (held) is left alone until the user releases it', () => {
  const queues = [{ active: true, maxConcurrent: 3, itemIds: [1, 2], held: [2] }];
  const map = rows([[1, 'completed'], [2, 'paused']]);
  assert.deepEqual(planQueueStarts(queues, map), []);
  setQueueHold(queues, 2, false);
  assert.deepEqual(planQueueStarts(queues, map), [2]);
});

test('a paused queue starts nothing', () => {
  const queues = [{ active: false, maxConcurrent: 2, itemIds: [1] }];
  assert.deepEqual(planQueueStarts(queues, rows([[1, 'paused']])), []);
});

test('the orchestrator starts nothing while an update install is in flight', () => {
  const queues = [{ active: true, maxConcurrent: 2, itemIds: [1, 2] }];
  const map = rows([[1, 'paused'], [2, 'paused']]);
  assert.deepEqual(planQueueStarts(queues, map), [1, 2]);
  assert.deepEqual(planQueueStarts(queues, map, { blocked: true }), []);
});

test('an item that is a member of two active queues is started once', () => {
  const queues = [
    { active: true, maxConcurrent: 2, itemIds: [1, 2] },
    { active: true, maxConcurrent: 4, itemIds: [1, 3] },
  ];
  const map = rows([[1, 'paused'], [2, 'queued'], [3, 'queued']]);
  assert.deepEqual(planQueueStarts(queues, map), [1, 2, 3]);
});

test('failed, cancelled and finished members are never restarted', () => {
  const queues = [{ active: true, maxConcurrent: 8, itemIds: [1, 2, 3, 4] }];
  const map = rows([[1, 'failed'], [2, 'cancelled'], [3, 'completed'], [4, 'merging']]);
  assert.deepEqual(planQueueStarts(queues, map), []);
});

test('a merging member still occupies a slot', () => {
  const queues = [{ active: true, maxConcurrent: 1, itemIds: [1, 2] }];
  const map = rows([[1, 'merging'], [2, 'paused']]);
  assert.deepEqual(planQueueStarts(queues, map), []);
});

test('rows that no longer exist (removed downloads) are ignored', () => {
  const queues = [{ active: true, maxConcurrent: 2, itemIds: [7, 8] }];
  const map = rows([[8, 'paused']]);
  assert.deepEqual(planQueueStarts(queues, map), [8]);
});

test('queues persisted before the hold existed behave as before', () => {
  const queues = [{ active: true, maxConcurrent: 2, itemIds: [1] }]; // no `held`
  assert.deepEqual(planQueueStarts(queues, rows([[1, 'paused']])), [1]);
});

test('setQueueHold records and clears the hold on every queue containing the item', () => {
  const a = { id: 'a', active: true, itemIds: [1, 2] };
  const b = { id: 'b', active: true, itemIds: [1, 3] };
  const queues = [a, b];
  assert.equal(setQueueHold(queues, 1, true), true);
  assert.deepEqual(a.held, [1]);
  assert.deepEqual(b.held, [1]);
  // idempotent
  assert.equal(setQueueHold(queues, 1, true), false);
  assert.equal(setQueueHold(queues, 99, true), false); // not a member anywhere
  assert.equal(setQueueHold(queues, 1, false), true);
  assert.deepEqual(a.held, []);
  assert.deepEqual(b.held, []);
  assert.equal(setQueueHold(queues, 1, false), false);
});
