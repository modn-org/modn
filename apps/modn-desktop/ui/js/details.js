function element(tag, className, text) {
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
 * Mod details stub. Later tasks fill the file plan, source, safety, versions
 * and history sections.
 * @param {HTMLElement} root
 * @param {{gameId: string, modId: string, path: string}} ctx
 */
export function render(root, ctx) {
  const view = element('div', 'modn-view modn-details');
  view.dataset.testid = 'mod-details';
  view.dataset.gameId = ctx.gameId;
  view.dataset.modId = ctx.modId;
  const header = element('header', 'modn-view__header');
  const titles = element('div', 'modn-row__text');
  titles.append(element('h1', undefined, ctx.modId));
  titles.append(element('p', 'modn-muted', ctx.gameId));
  header.append(titles);
  const card = element('section', 'modn-card');
  card.append(element('h2', undefined, 'Mod details'));
  card.append(
    element(
      'p',
      'modn-muted',
      'The file plan, source, safety and history sections arrive in a later screen.',
    ),
  );
  const back = element('a', 'modn-link', 'Back to library');
  back.href = ctx.path
    ? `#/game/${encodeURIComponent(ctx.gameId)}?path=${encodeURIComponent(ctx.path)}`
    : '#/';
  back.dataset.testid = 'back-to-library';
  view.append(header, card, back);
  root.append(view);
}
