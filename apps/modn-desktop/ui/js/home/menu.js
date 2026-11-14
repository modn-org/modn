/** The small Add menu that lists the supported game ids. */

import { gameLabel } from './data.js';
import { button, element } from './dom.js';

const MENU_ID = 'modn-home-add-menu';

/**
 * Builds the Add menu and wires the Add button to it.
 * @param {{anchor: HTMLButtonElement,
 *   container: HTMLElement,
 *   listGameIds: () => string[],
 *   onPick: (gameId: string) => void}} options
 * @returns {{close: () => void}}
 */
export function createAddMenu({ anchor, container, listGameIds, onPick }) {
  const menu = element('div', 'modn-menu');
  menu.id = MENU_ID;
  menu.dataset.testid = 'home-add-menu';
  menu.setAttribute('role', 'menu');
  menu.setAttribute('aria-label', 'Supported games');
  menu.hidden = true;
  container.append(menu);

  anchor.setAttribute('aria-haspopup', 'menu');
  anchor.setAttribute('aria-expanded', 'false');
  anchor.setAttribute('aria-controls', MENU_ID);

  const close = () => {
    if (menu.hidden) {
      return;
    }
    menu.hidden = true;
    anchor.setAttribute('aria-expanded', 'false');
    document.removeEventListener('pointerdown', onPointerDown, true);
    document.removeEventListener('keydown', onEscape, true);
  };

  /**
   * @param {PointerEvent} event
   */
  function onPointerDown(event) {
    const target = event.target;
    if (!(target instanceof Node)) {
      return;
    }
    if (menu.contains(target) || anchor.contains(target)) {
      return;
    }
    close();
  }

  /**
   * @param {KeyboardEvent} event
   */
  function onEscape(event) {
    if (event.key !== 'Escape') {
      return;
    }
    close();
    anchor.focus();
  }

  const open = () => {
    menu.replaceChildren();
    for (const gameId of listGameIds()) {
      const name = gameLabel(gameId);
      const item = button(name, `home-add-${gameId}`, `Add ${name} from this PC`, 'modn-menu__item');
      item.setAttribute('role', 'menuitem');
      item.addEventListener('click', () => {
        close();
        onPick(gameId);
      });
      menu.append(item);
    }
    menu.hidden = false;
    anchor.setAttribute('aria-expanded', 'true');
    document.addEventListener('pointerdown', onPointerDown, true);
    document.addEventListener('keydown', onEscape, true);
    const first = menu.querySelector('button');
    if (first) {
      first.focus();
    }
  };

  anchor.addEventListener('click', () => {
    if (menu.hidden) {
      open();
      return;
    }
    close();
  });

  menu.addEventListener('keydown', (event) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') {
      return;
    }
    const items = [...menu.querySelectorAll('button')];
    if (items.length === 0) {
      return;
    }
    event.preventDefault();
    const index = items.indexOf(/** @type {HTMLButtonElement} */ (document.activeElement));
    const next =
      index < 0
        ? event.key === 'ArrowDown'
          ? 0
          : items.length - 1
        : (index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
    items[next].focus();
  });

  return { close };
}
