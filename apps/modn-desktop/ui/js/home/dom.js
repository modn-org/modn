/** Small DOM helpers shared by the Home view modules. */

/**
 * Builds an element with an optional class name and text.
 * @param {string} tag
 * @param {string} [className]
 * @param {string} [text]
 * @returns {HTMLElement}
 */
export function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) {
    node.className = className;
  }
  if (text !== undefined) {
    node.textContent = text;
  }
  return node;
}

/**
 * Builds a button with a stable test id and an accessible name.
 * @param {string} label
 * @param {string} testId
 * @param {string} accessibleName
 * @param {string} [className]
 * @returns {HTMLButtonElement}
 */
export function button(label, testId, accessibleName, className = 'modn-button') {
  const node = /** @type {HTMLButtonElement} */ (element('button', className, label));
  node.type = 'button';
  node.dataset.testid = testId;
  node.setAttribute('aria-label', accessibleName);
  return node;
}

/**
 * Builds the polite status line under the view header.
 * @param {string} [message]
 * @returns {HTMLParagraphElement}
 */
export function statusLine(message = '') {
  const status = /** @type {HTMLParagraphElement} */ (
    element('p', 'modn-status', message)
  );
  status.setAttribute('role', 'status');
  return status;
}

/**
 * Replaces status text and error styling in one call.
 * @param {HTMLElement} node
 * @param {string} message
 * @param {boolean} [isError]
 */
export function setStatus(node, message, isError = false) {
  node.textContent = message;
  node.classList.toggle('modn-status--error', Boolean(isError));
}
