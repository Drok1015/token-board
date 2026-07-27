import './style.css';

const params = new URLSearchParams(window.location.search);

if (params.get('view') === 'settings') {
  import('./settings.js').then(({ mountSettings }) => mountSettings());
} else {
  window.addEventListener('contextmenu', (event) => {
    event.preventDefault();
    window.dispatchEvent(new CustomEvent('token-board-context-menu'));
  }, { capture: true });
  import('./board.js').then(({ mountBoard }) => mountBoard());
}
