import assert from 'node:assert/strict';
import test from 'node:test';

import { operationIdOf } from '../../ui/js/library/undo-target.js';

test('binds Undo to the top-level operationId of the write result', () => {
  assert.equal(operationIdOf({ operationId: 'op-9' }), 'op-9');
});

test('ignores package IDs and history-shaped fields', () => {
  assert.equal(operationIdOf({ package_id: 'pkg-1' }), '');
  assert.equal(operationIdOf({ installed: { id: 'pkg-1' } }), '');
  assert.equal(operationIdOf({ packages: [{ package_id_after: 'pkg-1' }] }), '');
});

test('rejects the legacy snake_case operation field', () => {
  assert.equal(operationIdOf({ operation_id: 'op-legacy' }), '');
});

test('returns an empty binding for absent or malformed results', () => {
  assert.equal(operationIdOf(null), '');
  assert.equal(operationIdOf(undefined), '');
  assert.equal(operationIdOf('op-9'), '');
  assert.equal(operationIdOf({ operationId: 9 }), '');
  assert.equal(operationIdOf({ operationId: '' }), '');
});
