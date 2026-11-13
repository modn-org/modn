import { planInstall } from '../api.js';
import { messageFor } from '../messages.js';
import { element } from './dom.js';

const BLOCKED_TEXT = 'Modn blocked this plan.';
const EMPTY_TEXT = 'This plan writes no files.';

/**
 * @param {unknown} report
 * @returns {Object|null}
 */
function planOf(report) {
  if (!report || typeof report !== 'object') {
    return null;
  }
  const plan = report.plan;
  return plan && typeof plan === 'object' ? plan : report;
}

/**
 * @param {Object|null} plan
 * @returns {Object[]}
 */
function operationsOf(plan) {
  return plan && Array.isArray(plan.operations) ? plan.operations : [];
}

/**
 * @param {unknown} report
 * @param {Object|null} plan
 * @returns {Object[]}
 */
function blockersOf(report, plan) {
  /** @type {Object[]} */
  const blockers = [];
  if (plan && Array.isArray(plan.blockers)) {
    blockers.push(...plan.blockers);
  }
  if (report && typeof report === 'object' && Array.isArray(report.blockers)) {
    blockers.push(...report.blockers);
  }
  return blockers.filter((blocker) => blocker && typeof blocker === 'object');
}

/**
 * Reads the approval the catalog plan reports as uncovered.
 * @param {unknown} report
 * @returns {string[]}
 */
function approvalNeeds(report) {
  if (!report || typeof report !== 'object') {
    return [];
  }
  const approval = report.approval;
  if (!approval || typeof approval !== 'object') {
    return [];
  }
  const uncovered =
    approval.uncovered && typeof approval.uncovered === 'object' ? approval.uncovered : {};
  /** @type {string[]} */
  const needs = [];
  if (uncovered.native_injector === true) {
    needs.push('native code');
  }
  const executables = Array.isArray(uncovered.executables) ? uncovered.executables.length : 0;
  if (executables > 0) {
    needs.push(executables === 1 ? '1 executable' : `${executables} executables`);
  }
  if (needs.length === 0 && approval.required === true) {
    needs.push('this mod');
  }
  return needs;
}

/**
 * @param {Object} operation
 * @returns {HTMLLIElement}
 */
function operationRow(operation) {
  const removing = operation.kind === 'remove' || operation.remove === true;
  const row = element('li', 'modn-plan__row');
  row.dataset.operation = removing ? 'remove' : 'write';
  const path = element(
    'span',
    'modn-plan__path',
    typeof operation.destination === 'string' ? operation.destination : '',
  );
  row.append(path, element('span', 'modn-plan__tag', removing ? 'Remove' : 'Write'));
  return row;
}

/**
 * @param {Object|null} plan
 * @param {Object[]} blockers
 * @param {string[]} needs
 * @param {HTMLElement} status
 * @param {HTMLElement} list
 * @param {HTMLElement} note
 */
function renderPlan(plan, blockers, needs, status, list, note) {
  const operations = operationsOf(plan);
  list.replaceChildren();
  if (operations.length === 0) {
    const empty = element('li', 'modn-muted', EMPTY_TEXT);
    empty.dataset.testid = 'details-plan-empty';
    list.append(empty);
  }
  for (const operation of operations) {
    if (operation && typeof operation === 'object') {
      list.append(operationRow(operation));
    }
  }
  for (const blocker of blockers) {
    const reason = element('li', 'modn-plan__blocker', messageFor(blocker.code));
    reason.dataset.code = typeof blocker.code === 'string' ? blocker.code : '';
    list.append(reason);
  }
  note.textContent =
    needs.length > 0 ? `Approval needed: ${needs.join(', ')}.` : 'No approval needed.';
  status.classList.toggle('modn-status--error', blockers.length > 0);
  if (blockers.length > 0) {
    status.textContent = BLOCKED_TEXT;
  } else {
    status.textContent = operations.length === 1 ? '1 file' : `${operations.length} files`;
  }
}

/**
 * Collapsed file plan. It reads the plan only when the reader opens the
 * section, and it never blocks or confirms the one-click operation.
 * @param {{gameId: string, modId: string, path: string}} ctx
 * @returns {HTMLDetailsElement}
 */
export function createPlanSection(ctx) {
  const section = document.createElement('details');
  section.className = 'modn-card modn-details__plan';
  section.dataset.testid = 'details-plan';
  const summary = element('summary', 'modn-details__summary', 'Details');
  const body = element('div', 'modn-details__body');
  const status = element('p', 'modn-status');
  status.setAttribute('role', 'status');
  const list = element('ul', 'modn-plan');
  list.setAttribute('aria-label', 'File plan');
  const note = element('p', 'modn-muted modn-details__approval');
  note.dataset.testid = 'details-plan-approval';
  body.append(status, list, note);
  section.append(summary, body);

  let loading = false;
  let loaded = false;

  async function loadPlan() {
    if (loading || loaded) {
      return;
    }
    if (!ctx.path) {
      status.textContent = 'Choose an installation first.';
      loaded = true;
      return;
    }
    loading = true;
    status.classList.remove('modn-status--error');
    status.textContent = 'Reading the file plan…';
    try {
      const report = await planInstall({
        gameId: ctx.gameId,
        path: ctx.path,
        modId: ctx.modId,
      });
      const plan = planOf(report);
      renderPlan(plan, blockersOf(report, plan), approvalNeeds(report), status, list, note);
      loaded = true;
    } catch (error) {
      status.textContent = messageFor(error);
      status.classList.add('modn-status--error');
    } finally {
      loading = false;
    }
  }

  section.addEventListener('toggle', () => {
    if (section.open) {
      void loadPlan();
    }
  });

  return section;
}
