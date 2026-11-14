import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { messageFor } from '../../ui/js/messages.js';

const UI_MESSAGES = fileURLToPath(new URL('../../ui/js/messages.js', import.meta.url));
const BACKEND_SOURCES = fileURLToPath(new URL('../../src', import.meta.url));

/**
 * Reads every Rust source under a directory.
 * @param {string} directory
 * @returns {string[]}
 */
function rustSources(directory) {
  const found = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      found.push(...rustSources(path));
    } else if (entry.name.endsWith('.rs')) {
      found.push(readFileSync(path, 'utf8'));
    }
  }
  return found;
}

test('an oversized result after the commit reads as a completed operation', () => {
  const expected = 'Done. Refresh to see the result.';
  assert.equal(messageFor('BACKEND_OUTPUT_TRUNCATED_AFTER_COMMIT'), expected);
  assert.equal(messageFor({ code: 'BACKEND_OUTPUT_TRUNCATED_AFTER_COMMIT' }), expected);
});

test('every backend code the UI names has a backend emitter', () => {
  const messages = readFileSync(UI_MESSAGES, 'utf8');
  const named = new Set(messages.match(/BACKEND_[A-Z0-9_]+/g) ?? []);
  assert.ok(named.size > 0, 'messages.js must name at least one backend code');
  const emitted = new Set(
    rustSources(BACKEND_SOURCES).flatMap((text) => text.match(/BACKEND_[A-Z0-9_]+/g) ?? []),
  );
  for (const code of named) {
    assert.ok(
      emitted.has(code),
      `${code} is named in messages.js but no backend source emits it`,
    );
  }
});

test('unknown codes keep the generic fallback', () => {
  assert.match(messageFor('NOT_A_REAL_CODE'), /Something went wrong/);
  assert.match(messageFor({ code: 'NOT_A_REAL_CODE' }), /Something went wrong/);
});
