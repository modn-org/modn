import assert from 'node:assert/strict';
import test from 'node:test';

import { messageFor } from '../../ui/js/messages.js';

test('both after-commit failures read as a completed operation', () => {
  for (const code of [
    'BACKEND_OUTPUT_TRUNCATED_AFTER_COMMIT',
    'BACKEND_STREAM_FAILED_AFTER_COMMIT',
  ]) {
    assert.equal(messageFor(code), 'Done. Refresh to see the result.');
    assert.equal(messageFor({ code }), 'Done. Refresh to see the result.');
  }
});

test('unknown codes keep the generic fallback', () => {
  assert.match(messageFor('NOT_A_REAL_CODE'), /Something went wrong/);
  assert.match(messageFor({ code: 'NOT_A_REAL_CODE' }), /Something went wrong/);
});
