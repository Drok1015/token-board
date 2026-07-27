import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Menu, MenuItem } from '@tauri-apps/api/menu';
import {
  currentMonitor,
  getCurrentWindow,
  LogicalPosition,
  LogicalSize,
} from '@tauri-apps/api/window';

const COMPACT_BOARD_WIDTH = 240;
const EXPANDED_BOARD_WIDTH = 300;
const BOARD_HEIGHT = 175;
const EDGE_TAB_WIDTH = 24;
const EDGE_TAB_HEIGHT = 72;
const SNAP_DISTANCE = 16;
const FADE_DURATION = 100;
const DEFAULT_SETTINGS = { autoHide: false, hideDelaySeconds: 10, showPlans: true };

export function mountBoard() {
  const app = document.querySelector('#app');
  const appWindow = getCurrentWindow();

  app.innerHTML = `
    <main class="app-shell" id="app-shell">
      <section class="board" aria-label="Token 看板" data-tauri-drag-region>
        <div class="screen" aria-live="polite" data-tauri-drag-region>
          <div class="screen-title">TOKEN 看板 <span class="updated" id="updated">自动刷新</span><span class="signal">●</span></div>
          <div class="quota-row"><b>CODEX</b><span class="plan-badge" id="codex-plan"></span><span class="quota-value" id="codex">读取中…</span></div>
          <div class="quota-row"><b>KIMI</b><span class="plan-badge" id="kimi-plan"></span><span class="quota-value" id="kimi">读取中…</span></div>
          <div class="quota-row"><b>GLM</b><span class="plan-badge" id="glm-plan"></span><span class="quota-value" id="glm">读取中…</span></div>
          <div class="quota-row"><b>DEEPSEEK</b><span class="plan-badge" id="deepseek-plan"></span><span class="quota-value" id="deepseek">读取中…</span></div>
        </div>
      </section>
      <button class="edge-tab" id="edge-tab" type="button" aria-label="展开 Token 看板" title="点击展开；上下拖动调整位置">›</button>
    </main>`;

  const ids = { CODEX: 'codex', KIMI: 'kimi', GLM: 'glm', DEEPSEEK: 'deepseek' };
  const planIds = { CODEX: 'codex-plan', KIMI: 'kimi-plan', GLM: 'glm-plan', DEEPSEEK: 'deepseek-plan' };
  const shell = document.querySelector('#app-shell');
  const edgeTab = document.querySelector('#edge-tab');
  const updated = document.querySelector('#updated');
  let edgeState = null;
  let transitioning = false;
  let edgePointer = null;
  let ignoreTabClick = false;
  let settings = { ...DEFAULT_SETTINGS };
  let autoHideTimer = null;
  let refreshInProgress = false;

  const clamp = (value, min, max) => Math.min(Math.max(value, min), max);
  const wait = (milliseconds) => new Promise((resolve) => { setTimeout(resolve, milliseconds); });

  function clearAutoHideTimer() {
    if (autoHideTimer !== null) {
      window.clearTimeout(autoHideTimer);
      autoHideTimer = null;
    }
  }

  function currentBoardWidth() {
    return settings.showPlans ? EXPANDED_BOARD_WIDTH : COMPACT_BOARD_WIDTH;
  }

  async function resizeBoardForSettings() {
    if (edgeState || transitioning) return;
    const width = currentBoardWidth();
    const monitor = await currentMonitor();
    if (!monitor) {
      await appWindow.setSize(new LogicalSize(width, BOARD_HEIGHT));
      return;
    }

    const scale = monitor.scaleFactor;
    const workPosition = monitor.workArea.position.toLogical(scale);
    const workSize = monitor.workArea.size.toLogical(scale);
    const position = (await appWindow.outerPosition()).toLogical(scale);
    const size = (await appWindow.outerSize()).toLogical(scale);
    if (Math.abs(size.width - width) < 1) return;

    const leftDistance = Math.abs(position.x - workPosition.x);
    const rightDistance = Math.abs(workPosition.x + workSize.width - (position.x + size.width));
    transitioning = true;
    try {
      await appWindow.setSize(new LogicalSize(width, BOARD_HEIGHT));
      if (rightDistance < leftDistance) {
        const x = clamp(
          position.x + size.width - width,
          workPosition.x,
          workPosition.x + workSize.width - width,
        );
        await appWindow.setPosition(new LogicalPosition(x, position.y));
      }
    } finally {
      transitioning = false;
    }
  }

  async function applySettings(value) {
    settings = { ...DEFAULT_SETTINGS, ...value };
    shell.classList.toggle('plans-hidden', !settings.showPlans);
    await resizeBoardForSettings();
    scheduleAutoHide();
  }

  function scheduleAutoHide() {
    clearAutoHideTimer();
    if (!settings.autoHide || edgeState) return;
    const delay = clamp(Number(settings.hideDelaySeconds) || 10, 1, 3600) * 1000;
    autoHideTimer = window.setTimeout(() => {
      autoHideTimer = null;
      collapseToNearestEdge().catch(console.error);
    }, delay);
  }

  async function collapseAtEdge(side, monitor, physicalPosition) {
    if (edgeState || transitioning) return;
    clearAutoHideTimer();
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
    shell.classList.add('edge-switching');

    try {
      await wait(FADE_DURATION);
      shell.classList.add('edge-hidden', `edge-${side}`);
      edgeTab.textContent = side === 'left' ? '›' : '‹';
      edgeTab.setAttribute('aria-label', `展开${side === 'left' ? '左侧' : '右侧'} Token 看板`);
      await appWindow.setSize(new LogicalSize(EDGE_TAB_WIDTH, EDGE_TAB_HEIGHT));
      const tabX = side === 'left'
        ? workPosition.x
        : workPosition.x + workSize.width - EDGE_TAB_WIDTH;
      await appWindow.setPosition(new LogicalPosition(tabX, tabY));
    } finally {
      await wait(30);
      shell.classList.remove('edge-switching');
      transitioning = false;
    }
  }

  async function collapseToNearestEdge() {
    if (edgeState) return;
    if (transitioning) {
      autoHideTimer = window.setTimeout(() => {
        autoHideTimer = null;
        collapseToNearestEdge().catch(console.error);
      }, 250);
      return;
    }
    const monitor = await currentMonitor();
    if (!monitor) return;
    const scale = monitor.scaleFactor;
    const workPosition = monitor.workArea.position.toLogical(scale);
    const workSize = monitor.workArea.size.toLogical(scale);
    const physicalPosition = await appWindow.outerPosition();
    const position = physicalPosition.toLogical(scale);
    const size = (await appWindow.outerSize()).toLogical(scale);
    const leftDistance = Math.abs(position.x - workPosition.x);
    const rightDistance = Math.abs(workPosition.x + workSize.width - (position.x + size.width));
    await collapseAtEdge(leftDistance <= rightDistance ? 'left' : 'right', monitor, physicalPosition);
  }

  async function pinEdgeTab(physicalPosition) {
    if (!edgeState || transitioning) return;
    const {
      side, workPosition, workSize, scale,
    } = edgeState;
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
    const {
      side, workPosition, workSize, tabY,
    } = edgeState;
    shell.classList.add('edge-switching');
    try {
      await wait(FADE_DURATION);
      const width = currentBoardWidth();
      await appWindow.setSize(new LogicalSize(width, BOARD_HEIGHT));
      const x = side === 'left'
        ? workPosition.x + SNAP_DISTANCE + 10
        : workPosition.x + workSize.width - width - SNAP_DISTANCE - 10;
      const y = clamp(
        tabY - (BOARD_HEIGHT - EDGE_TAB_HEIGHT) / 2,
        workPosition.y,
        workPosition.y + workSize.height - BOARD_HEIGHT,
      );
      await appWindow.setPosition(new LogicalPosition(x, y));
      shell.classList.remove('edge-hidden', `edge-${side}`);
      edgeState = null;
    } finally {
      await wait(30);
      shell.classList.remove('edge-switching');
      transitioning = false;
    }
    scheduleAutoHide();
  }

  async function refreshQuotas() {
    if (refreshInProgress) return;
    refreshInProgress = true;
    updated.textContent = '刷新中…';
    try {
      const lines = await invoke('get_quotas');
      lines.forEach(({ provider, value, plan }) => {
        const node = document.querySelector(`#${ids[provider]}`);
        if (node) node.textContent = value;
        const planNode = document.querySelector(`#${planIds[provider]}`);
        if (planNode) planNode.textContent = plan || '';
      });
      updated.textContent = '刚刚刷新';
    } catch (error) {
      console.error(error);
      Object.values(ids).forEach((id) => {
        document.querySelector(`#${id}`).textContent = '读取失败';
      });
      updated.textContent = '刷新失败';
    } finally {
      refreshInProgress = false;
    }
  }

  async function createContextMenu() {
    const refreshItem = await MenuItem.new({
      id: 'refresh',
      text: '刷新',
      action: () => { refreshQuotas().catch(console.error); },
    });
    const settingsItem = await MenuItem.new({
      id: 'settings',
      text: '设置',
      action: () => { invoke('open_settings').catch(console.error); },
    });
    const closeItem = await MenuItem.new({
      id: 'close',
      text: '关闭',
      action: () => { invoke('close_app').catch(console.error); },
    });
    return Menu.new({ items: [refreshItem, settingsItem, closeItem] });
  }

  edgeTab.addEventListener('pointerdown', (event) => {
    if (!edgeState) return;
    edgePointer = {
      id: event.pointerId,
      startScreenY: event.screenY,
      startTabY: edgeState.tabY,
      moved: false,
    };
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
    revealBoard().catch(console.error);
  });

  appWindow.onMoved(({ payload }) => {
    const task = edgeState ? pinEdgeTab(payload) : checkSnap(payload);
    task.catch(console.error);
  });

  const contextMenuPromise = createContextMenu();
  window.addEventListener('token-board-context-menu', async () => {
    clearAutoHideTimer();
    try {
      const contextMenu = await contextMenuPromise;
      await contextMenu.popup();
    } catch (error) {
      console.error(error);
    } finally {
      scheduleAutoHide();
    }
  });

  listen('settings-updated', ({ payload }) => {
    applySettings(payload).catch(console.error);
  }).catch(console.error);

  invoke('get_settings')
    .then((value) => applySettings(value))
    .catch(console.error);

  refreshQuotas().catch(console.error);
  window.setInterval(() => { refreshQuotas().catch(console.error); }, 5 * 60 * 1000);
}
