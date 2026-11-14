import assert from 'node:assert/strict';
import test from 'node:test';

import { createOperationButton } from '../../ui/js/op-button.js';
import {
  deferred,
  findAllByClass,
  findByTestId,
  find,
  flush,
  installDom,
  installWindow,
  lastToast,
  toastHost,
} from './support/fake-dom.mjs';

/**
 * @param {Object} [overrides]
 */
function makeButton(overrides = {}) {
  const document = installDom();
  const askHost = document.createElement('div');
  const button = createOperationButton({
    label: 'Install',
    action: 'Install',
    askHost,
    ...overrides,
  });
  document.body.append(button.element);
  return { button, askHost, document };
}

test('reserves the run before the first await', async () => {
  installWindow();
  const preparing = deferred();
  let runs = 0;
  const { button } = makeButton({
    prepare: () => preparing.promise,
    run: async () => {
      runs += 1;
      return { operationId: 'op-1' };
    },
  });
  const first = button.start();
  assert.equal(button.button.dataset.state, 'running');
  assert.equal(button.button.textContent, 'Preparing…');
  assert.equal(button.button.disabled, true);
  const second = button.start();
  preparing.resolve({});
  await Promise.all([first, second]);
  assert.equal(runs, 1, 'a second click during prepare must not start another run');
});

test('records an early cancel and skips the write', async () => {
  const progress = deferred();
  const cancelled = [];
  installWindow({
    invoke: async (command, args) => {
      if (command === 'cancel_run') {
        cancelled.push(args);
      }
      return {};
    },
    listen: () => progress.promise,
  });
  let runs = 0;
  const { button } = makeButton({
    run: async () => {
      runs += 1;
      return { operationId: 'op-1' };
    },
  });
  const running = button.start();
  await flush();
  assert.equal(button.button.textContent, 'Cancel');
  assert.equal(button.button.disabled, false);
  button.button.click();
  assert.equal(button.button.textContent, 'Cancelling…');
  assert.deepEqual(cancelled, [], 'cancel_run must wait until the write is registered');
  assert.equal(runs, 0, 'the write must not start after an early cancel');
  progress.resolve(() => {});
  await running;
  assert.equal(runs, 0);
  assert.equal(button.button.dataset.state, 'idle');
  assert.equal(lastToast().textContent, 'Cancelled. Nothing changed.');
});

test('cancels the registered run after the write starts', async () => {
  const cancelled = [];
  installWindow({
    invoke: async (command, args) => {
      if (command === 'cancel_run') {
        cancelled.push(args.runId);
      }
      return {};
    },
  });
  const writing = deferred();
  let runId = '';
  const { button } = makeButton({
    run: async (args) => {
      runId = args.runId;
      return writing.promise;
    },
  });
  const running = button.start();
  await flush();
  button.button.click();
  await flush();
  assert.deepEqual(cancelled, [runId], 'a running write receives cancel_run');
  assert.equal(button.button.textContent, 'Cancelling…');
  writing.resolve({ operationId: 'op-2' });
  await running;
  assert.equal(button.button.dataset.state, 'idle');
});

test('a destroyed button ignores its settled run', async () => {
  let unlistenCount = 0;
  installWindow({
    listen: async () => () => {
      unlistenCount += 1;
    },
  });
  const writing = deferred();
  const { button } = makeButton({ run: () => writing.promise });
  const running = button.start();
  await flush();
  button.destroy();
  writing.resolve({ operationId: 'op-3' });
  await running;
  assert.equal(unlistenCount, 1);
  assert.equal(toastHost(), null, 'a destroyed run must not toast');
  assert.equal(button.element.parentNode, null);
});

test('one Allow approves the native injector and every planned executable', async () => {
  installWindow();
  const calls = [];
  let planCalls = 0;
  const { button, askHost } = makeButton({
    plan: async () => {
      planCalls += 1;
      return {
        approval: {
          required: true,
          uncovered: {
            native_injector: true,
            executables: ['C:/game/bin/tool.exe'],
          },
        },
      };
    },
    run: async ({ approvals }) => {
      calls.push(approvals);
      if (calls.length === 1) {
        throw {
          code: 'APPROVAL_REQUIRED',
          message: 'approval required',
          approval: { nativeInjector: true, executables: [], complete: false },
        };
      }
      return { operationId: 'op-4' };
    },
  });
  await button.start();
  assert.equal(calls.length, 1);
  assert.equal(planCalls, 1, 'an incomplete refusal must read the plan');
  assert.ok(findByTestId(askHost, 'ask-allow'), 'the ask must render in its host');
  assert.match(askHost.textContent, /native injector/);
  assert.match(askHost.textContent, /C:\/game\/bin\/tool\.exe/);
  findByTestId(askHost, 'ask-allow').click();
  await flush();
  assert.equal(calls.length, 2, 'one Allow retries the write once');
  assert.deepEqual(calls[1], {
    native: true,
    executables: ['C:/game/bin/tool.exe'],
  });
});

test('the ask lists each executable and the native permission separately', async () => {
  installWindow();
  const { button, askHost } = makeButton({
    run: async () => {
      throw {
        code: 'APPROVAL_REQUIRED',
        approval: {
          nativeInjector: true,
          executables: ['C:/tools/one.exe', 'C:/tools/two.exe'],
          complete: true,
        },
      };
    },
  });
  await button.start();
  const lines = findAllByClass(askHost, 'modn-ask__question');
  const nativeLine = lines.find((line) => line.textContent.includes('native injector'));
  assert.ok(nativeLine, 'the native permission needs its own line');
  assert.doesNotMatch(nativeLine.textContent, /one\.exe/, 'paths must not share that line');
  const paths = findAllByClass(askHost, 'modn-ask__path');
  assert.deepEqual(
    paths.map((path) => path.textContent),
    ['C:/tools/one.exe', 'C:/tools/two.exe'],
  );
});

test('an oversized result after the commit counts as a completed operation', async () => {
  const code = 'BACKEND_OUTPUT_TRUNCATED_AFTER_COMMIT';
  installWindow();
  let settled = 0;
  const { button } = makeButton({
    onSettled: () => {
      settled += 1;
    },
    run: async () => {
      throw { code };
    },
  });
  await button.start();
  assert.equal(button.button.dataset.state, 'idle');
  assert.equal(lastToast().textContent, 'Done. Refresh to see the result.');
  assert.equal(settled, 1, `${code} must refresh like a commit`);
});

test('a result without operationId gets no Undo action', async () => {
  installWindow();
  const { button } = makeButton({
    run: async () => ({ package_id: 'pkg-1', operation_id: 'op-legacy' }),
  });
  await button.start();
  assert.equal(lastToast().textContent, 'Install done');
  assert.equal(findByTestId(toastHost(), 'toast-undo'), null);
  assert.ok(find(toastHost(), (node) => node.dataset.testid === 'toast'));
});

test('a result with operationId binds the toast Undo', async () => {
  installWindow();
  const { button } = makeButton({
    run: async () => ({ operationId: 'op-7' }),
  });
  await button.start();
  assert.ok(findByTestId(toastHost(), 'toast-undo'), 'a bound result offers Undo');
});
