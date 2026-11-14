import assert from 'node:assert/strict';
import test from 'node:test';

import { createUndoLauncher } from '../../ui/js/library/tile.js';
import {
  findByTestId,
  flush,
  installDom,
  installWindow,
  lastToast,
} from './support/fake-dom.mjs';

/**
 * Installs a toast Undo host and returns its launcher.
 * @param {(args: Object) => Promise<unknown>} runUndo
 * @param {() => void} [onSettled]
 */
function makeLauncher(runUndo, onSettled = () => {}) {
  const document = installDom();
  const host = document.createElement('div');
  document.body.append(host);
  const launch = createUndoLauncher({ host, runUndo, onSettled });
  return { host, launch };
}

test('a blocked toast Undo removes its temporary row', async () => {
  installWindow();
  const calls = [];
  const { host, launch } = makeLauncher(async (args) => {
    calls.push(args);
    throw { code: 'CONFLICT', message: 'another package owns these files' };
  });

  launch('op-blocked');
  await flush();

  assert.equal(calls.length, 1);
  assert.equal(
    host.childElementCount,
    0,
    'a blocked Undo must not leave its temporary row in the Library',
  );
});

test('a declined ask removes its row and a retry keeps the operation ID', async () => {
  installWindow();
  const calls = [];
  const { host, launch } = makeLauncher(async (args) => {
    calls.push(args);
    if (calls.length === 1) {
      throw {
        code: 'APPROVAL_REQUIRED',
        approval: { nativeInjector: true, executables: [], complete: false },
      };
    }
    return { operationId: 'op-undo' };
  });

  launch('op-declined');
  await flush();

  const undo = findByTestId(host, 'toast-undo-button');
  assert.ok(undo, 'the temporary row offers an Undo button');
  assert.ok(findByTestId(host, 'ask-decline'), 'the approval prompt waits for a decision');
  findByTestId(host, 'ask-decline').click();
  await flush();

  assert.equal(calls.length, 1, 'a declined ask does not run the write');
  assert.equal(
    host.childElementCount,
    0,
    'a declined ask must not leave its temporary row in the Library',
  );

  undo.click();
  await flush();

  assert.equal(calls.length, 2, 'the idle Undo button still retries');
  assert.equal(
    calls[1].operationId,
    'op-declined',
    'a retry must undo the same operation the toast bound',
  );
});

test('a cancelled toast Undo removes its temporary row', async () => {
  installWindow();
  let settled = 0;
  const { host, launch } = makeLauncher(
    async () => {
      throw { code: 'CANCELLED', message: 'cancelled' };
    },
    () => {
      settled += 1;
    },
  );

  launch('op-cancelled');
  await flush();

  assert.equal(settled, 1, 'a cancelled Undo still refreshes the view');
  assert.equal(lastToast().textContent, 'Cancelled. Nothing changed.');
  assert.equal(
    host.childElementCount,
    0,
    'a cancelled Undo must not leave its temporary row in the Library',
  );
});
