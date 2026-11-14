import assert from 'node:assert/strict';
import test from 'node:test';

import { approvalsFrom } from '../../ui/js/library/approvals.js';

const PLAN = {
  approval: {
    required: true,
    uncovered: {
      native_injector: true,
      executables: ['C:/game/bin/one.exe', 'C:/game/bin/two.exe'],
    },
  },
};

test('a missing refusal reads every capability from the plan', async () => {
  let planCalls = 0;
  const approvals = await approvalsFrom({ code: 'APPROVAL_REQUIRED' }, 'APPROVAL_REQUIRED', async () => {
    planCalls += 1;
    return PLAN;
  });
  assert.equal(planCalls, 1);
  assert.deepEqual(approvals, {
    native: true,
    executables: ['C:/game/bin/one.exe', 'C:/game/bin/two.exe'],
  });
});

test('a refusal that lacks planned executables keeps the plan paths', async () => {
  const approvals = await approvalsFrom(
    {
      code: 'APPROVAL_REQUIRED',
      approval: { nativeInjector: true, executables: ['C:/game/bin/one.exe'] },
    },
    'APPROVAL_REQUIRED',
    async () => PLAN,
  );
  assert.deepEqual(approvals, {
    native: true,
    executables: ['C:/game/bin/one.exe', 'C:/game/bin/two.exe'],
  });
});

test('a complete refusal still merges what the plan adds without duplicates', async () => {
  const approvals = await approvalsFrom(
    {
      code: 'APPROVAL_REQUIRED',
      approval: {
        nativeInjector: false,
        executables: ['C:/game/bin/one.exe'],
        complete: true,
      },
    },
    'APPROVAL_REQUIRED',
    async () => PLAN,
  );
  assert.deepEqual(approvals, {
    native: true,
    executables: ['C:/game/bin/one.exe', 'C:/game/bin/two.exe'],
  });
});

test('a failing plan leaves the refusal data intact', async () => {
  const approvals = await approvalsFrom(
    {
      code: 'NATIVE_INJECTOR_NOT_ALLOWED',
      approval: { nativeInjector: true, executables: [] },
    },
    'NATIVE_INJECTOR_NOT_ALLOWED',
    async () => {
      throw new Error('plan failed');
    },
  );
  assert.deepEqual(approvals, { native: true, executables: [] });
});

test('no plan keeps only the refusal data', async () => {
  const approvals = await approvalsFrom(
    { approval: { nativeInjector: false, executables: ['C:/game/bin/one.exe'] } },
    'APPROVAL_REQUIRED',
    null,
  );
  assert.deepEqual(approvals, {
    native: false,
    executables: ['C:/game/bin/one.exe'],
  });
});
