import { invoke } from '@tauri-apps/api/core';
import { currentMonitor, getCurrentWindow, LogicalPosition, LogicalSize } from '@tauri-apps/api/window';
import './style.css';

const app = document.querySelector('#app');
const appWindow = getCurrentWindow();
const BOARD_WIDTH = 236;
const BOARD_HEIGHT = 175;
const EDGE_TAB_WIDTH = 24;
const EDGE_TAB_HEIGHT = 72;
const SNAP_DISTANCE = 16;

app.innerHTML = `
  <main class="app-shell" id="app-shell">
    <section class="board" aria-label="Token 看板" data-tauri-drag-region>
      <div class="screen" aria-live="polite" data-tauri-drag-region>
        <div class="screen-title">TOKEN 看板 <span class="updated">自动刷新</span><span class="signal">●</span></div>
        <div class="quota-row"><b>CODEX</b><span id="codex">读取中…</span></div>
        <div class="quota-row"><b>KIMI</b><span id="kimi">读取中…</span></div>
        <div class="quota-row"><b>GLM</b><span id="glm">读取中…</span></div>
        <div class="quota-row"><b>DEEPSEEK</b><span id="deepseek">读取中…</span></div>
      </div>
    </section>
    <button class="edge-tab" id="edge-tab" type="button" aria-label="展开 Token 看板" title="点击展开；上下拖动调整位置">›</button>
  </main>`;

const ids = { CODEX: 'codex', KIMI: 'kimi', GLM: 'glm', DEEPSEEK: 'deepseek' };
const shell = document.querySelector('#app-shell');
const edgeTab = document.querySelector('#edge-tab');
let edgeState = null;
let transitioning = false;
let edgePointer = null;
let ignoreTabClick = false;

const clamp = (value, min, max) => Math.min(Math.max(value, min), max);

async function collapseAtEdge(side, monitor, physicalPosition) {
  if (edgeState || transitioning) return;
  transitioning = true;
  const scale = monitor.scaleFactor;
  const workPosition = monitor.workArea.position.toLogical(scale);
  const workSize = monitor.workArea.size.toLogical(scale);
  const position = physicalPosition.toLogical(scale);
  const size = (await appWindow.outerSize()).toLogical(scale);
  const tabY = clamp(
    position.y + (size.height - EDGE_TAB_HEIGHT) / 2,
    workPosition.y,
    workPosition.y + workSize.height - EDGE_TAB_HEIGHT,
  );
  edgeState = { side, workPosition, workSize, tabY, scale };
  shell.classList.add('edge-hidden', `edge-${side}`);
  edgeTab.textContent = side === 'left' ? '›' : '‹';
  edgeTab.setAttribute('aria-label', `展开${side === 'left' ? '左侧' : '右侧'} Token 看板`);

  try {
    await appWindow.setSize(new LogicalSize(EDGE_TAB_WIDTH, EDGE_TAB_HEIGHT));
    const tabX = side === 'left'
      ? workPosition.x
      : workPosition.x + workSize.width - EDGE_TAB_WIDTH;
    await appWindow.setPosition(new LogicalPosition(tabX, tabY));
  } finally {
    transitioning = false;
  }
}

async function pinEdgeTab(physicalPosition) {
  if (!edgeState || transitioning) return;
  const { side, workPosition, workSize, scale } = edgeState;
  const position = physicalPosition.toLogical(scale);
  const tabY = clamp(position.y, workPosition.y, workPosition.y + workSize.height - EDGE_TAB_HEIGHT);
  edgeState.tabY = tabY;
  const tabX = side === 'left'
    ? workPosition.x
    : workPosition.x + workSize.width - EDGE_TAB_WIDTH;
  if (Math.abs(position.x - tabX) < 1 && Math.abs(position.y - tabY) < 1) return;
  transitioning = true;
  try {
    await appWindow.setPosition(new LogicalPosition(tabX, tabY));
  } finally {
    transitioning = false;
  }
}

async function moveEdgeTab(tabY) {
  if (!edgeState || transitioning) return;
  const { side, workPosition, workSize } = edgeState;
  edgeState.tabY = clamp(tabY, workPosition.y, workPosition.y + workSize.height - EDGE_TAB_HEIGHT);
  const tabX = side === 'left'
    ? workPosition.x
    : workPosition.x + workSize.width - EDGE_TAB_WIDTH;
  transitioning = true;
  try {
    await appWindow.setPosition(new LogicalPosition(tabX, edgeState.tabY));
  } finally {
    transitioning = false;
  }
}

async function checkSnap(physicalPosition) {
  if (edgeState || transitioning) return;
  const monitor = await currentMonitor();
  if (!monitor) return;
  const scale = monitor.scaleFactor;
  const workPosition = monitor.workArea.position.toLogical(scale);
  const workSize = monitor.workArea.size.toLogical(scale);
  const position = physicalPosition.toLogical(scale);
  const size = (await appWindow.outerSize()).toLogical(scale);
  const leftGap = position.x - workPosition.x;
  const rightGap = workPosition.x + workSize.width - (position.x + size.width);
  if (leftGap <= SNAP_DISTANCE) await collapseAtEdge('left', monitor, physicalPosition);
  else if (rightGap <= SNAP_DISTANCE) await collapseAtEdge('right', monitor, physicalPosition);
}

async function revealBoard() {
  if (!edgeState || transitioning) return;
  transitioning = true;
  const { side, workPosition, workSize, tabY } = edgeState;
  shell.classList.remove('edge-hidden', `edge-${side}`);
  try {
    await appWindow.setSize(new LogicalSize(BOARD_WIDTH, BOARD_HEIGHT));
    const x = side === 'left'
      ? workPosition.x + SNAP_DISTANCE + 10
      : workPosition.x + workSize.width - BOARD_WIDTH - SNAP_DISTANCE - 10;
    const y = clamp(
      tabY - (BOARD_HEIGHT - EDGE_TAB_HEIGHT) / 2,
      workPosition.y,
      workPosition.y + workSize.height - BOARD_HEIGHT,
    );
    await appWindow.setPosition(new LogicalPosition(x, y));
    edgeState = null;
  } finally {
    transitioning = false;
  }
}

edgeTab.addEventListener('pointerdown', (event) => {
  if (!edgeState) return;
  edgePointer = { id: event.pointerId, startScreenY: event.screenY, startTabY: edgeState.tabY, moved: false };
  edgeTab.setPointerCapture(event.pointerId);
});
edgeTab.addEventListener('pointermove', (event) => {
  if (!edgePointer || edgePointer.id !== event.pointerId) return;
  const deltaY = event.screenY - edgePointer.startScreenY;
  if (Math.abs(deltaY) > 3) edgePointer.moved = true;
  if (edgePointer.moved) moveEdgeTab(edgePointer.startTabY + deltaY).catch(console.error);
});
edgeTab.addEventListener('pointerup', (event) => {
  if (!edgePointer || edgePointer.id !== event.pointerId) return;
  ignoreTabClick = edgePointer.moved;
  edgePointer = null;
  edgeTab.releasePointerCapture(event.pointerId);
});
edgeTab.addEventListener('click', () => {
  if (ignoreTabClick) {
    ignoreTabClick = false;
    return;
  }
  revealBoard();
});
appWindow.onMoved(({ payload }) => {
  const task = edgeState ? pinEdgeTab(payload) : checkSnap(payload);
  task.catch(console.error);
});

async function refreshQuotas() {
  try {
    const lines = await invoke('get_quotas');
    lines.forEach(({ provider, value }) => {
      const node = document.querySelector(`#${ids[provider]}`);
      if (node) node.textContent = value;
    });
  } catch (error) {
    console.error(error);
    Object.values(ids).forEach((id) => { document.querySelector(`#${id}`).textContent = '读取失败'; });
  }
}

refreshQuotas();
setInterval(refreshQuotas, 5 * 60 * 1000);
