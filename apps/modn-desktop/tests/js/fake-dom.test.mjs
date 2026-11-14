import assert from 'node:assert/strict';
import test from 'node:test';

import { installDom } from './support/fake-dom.mjs';

test('clearing textContent detaches the removed children', () => {
  const document = installDom();
  const parent = document.createElement('div');
  const child = document.createElement('p');
  parent.append(child);

  parent.textContent = '';

  assert.equal(parent.childElementCount, 0);
  assert.equal(
    child.parentNode,
    null,
    'the browser detaches a child that textContent clears',
  );
});

test('click on a disabled button does nothing', () => {
  const document = installDom();
  const button = document.createElement('button');
  let clicks = 0;
  button.addEventListener('click', () => {
    clicks += 1;
  });

  button.disabled = true;
  button.click();
  assert.equal(clicks, 0, 'a disabled button must not dispatch a click');

  button.disabled = false;
  button.click();
  assert.equal(clicks, 1, 'an enabled button still dispatches its click');
});
