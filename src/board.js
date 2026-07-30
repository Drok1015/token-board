import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Menu, MenuItem } from '@tauri-apps/api/menu';
import { check as checkUpdate } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import {
  currentMonitor,
  getCurrentWindow,
  LogicalPosition,
  LogicalSize,
} from '@tauri-apps/api/window';

const COMPACT_BOARD_WIDTH = 240;
const EXPANDED_BOARD_WIDTH = 300;
const BOARD_HEIGHT = 175;
const ROW_HEIGHT = 25;
const EDGE_TAB_WIDTH = 24;
const EDGE_TAB_HEIGHT = 72;
const SNAP_DISTANCE = 16;
const FADE_DURATION = 100;
const DEFAULT_SETTINGS = {
  autoHide: false,
  hideDelaySeconds: 10,
  showPlans: true,
  visibleProviders: ['CODEX', 'KIMI', 'GLM', 'DEEPSEEK'],
};

export function mountBoard() {
  const app = document.querySelector('#app');
  const appWindow = getCurrentWindow();

  app.innerHTML = `
    <main class="app-shell" id="app-shell">
      <section class="board" aria-label="Token 看板" data-tauri-drag-region>
        <div class="screen" aria-live="polite" data-tauri-drag-region>
          <div class="screen-title">TOKEN 看板 <span class="signal">●</span><span class="updated" id="updated">自动刷新</span></div>
          <div class="quota-row" data-provider="CODEX"><b>CODEX</b><span class="plan-badge" id="codex-plan"></span><span class="quota-value" id="codex">读取中…</span></div>
          <div class="quota-row" data-provider="KIMI"><b>KIMI</b><span class="plan-badge" id="kimi-plan"></span><span class="quota-value" id="kimi">读取中…</span></div>
          <div class="quota-row" data-provider="GLM"><b>GLM</b><span class="plan-badge" id="glm-plan"></span><span class="quota-value" id="glm">读取中…</span></div>
          <div class="quota-row" data-provider="DEEPSEEK"><b>DEEPSEEK</b><span class="plan-badge" id="deepseek-plan"></span><span class="quota-value" id="deepseek">读取中…</span></div>
          <div class="screen-footer">5分钟刷新一次，可右键手动刷新</div>
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
  let codexFullAlerted = false;
  let lastRefreshAt = null;

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

  function visibleProviders() {
    const list = (settings.visibleProviders || []).filter((name) => name in ids);
    return list.length > 0 ? list : DEFAULT_SETTINGS.visibleProviders;
  }

  function currentBoardHeight() {
    return BOARD_HEIGHT - (DEFAULT_SETTINGS.visibleProviders.length - visibleProviders().length) * ROW_HEIGHT;
  }

  async function resizeBoardForSettings() {
    if (edgeState || transitioning) return;
    const width = currentBoardWidth();
    const height = currentBoardHeight();
    const monitor = await currentMonitor();
    if (!monitor) {
      await appWindow.setSize(new LogicalSize(width, height));
      return;
    }

    const scale = monitor.scaleFactor;
    const workPosition = monitor.workArea.position.toLogical(scale);
    const workSize = monitor.workArea.size.toLogical(scale);
    const position = (await appWindow.outerPosition()).toLogical(scale);
    const size = (await appWindow.outerSize()).toLogical(scale);
    if (Math.abs(size.width - width) < 1 && Math.abs(size.height - height) < 1) return;

    const leftDistance = Math.abs(position.x - workPosition.x);
    const rightDistance = Math.abs(workPosition.x + workSize.width - (position.x + size.width));
    transitioning = true;
    try {
      await appWindow.setSize(new LogicalSize(width, height));
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
    const visible = visibleProviders();
    document.querySelectorAll('.quota-row[data-provider]').forEach((row) => {
      row.classList.toggle('provider-hidden', !visible.includes(row.dataset.provider));
    });
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
      const height = currentBoardHeight();
      await appWindow.setSize(new LogicalSize(width, height));
      const x = side === 'left'
        ? workPosition.x + SNAP_DISTANCE + 10
        : workPosition.x + workSize.width - width - SNAP_DISTANCE - 10;
      const y = clamp(
        tabY - (height - EDGE_TAB_HEIGHT) / 2,
        workPosition.y,
        workPosition.y + workSize.height - height,
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

  // CODEX 只看 7d 额度窗口：回到 100% 时弹系统提醒一次；低于 100% 后解除锁存，再次回到 100% 会重新提醒
  async function maybeAlertCodexFull(lines) {
    const codex = lines.find((line) => line.provider === 'CODEX');
    if (!codex) return;
    const match = codex.value.match(/7d\s+(\d+)%/);
    if (!match) return; // 没有 7d 数据或读取失败时不改变锁存状态
    if (Number(match[1]) >= 100) {
      if (codexFullAlerted) return;
      codexFullAlerted = true;
      await invoke('notify_codex_full');
    } else {
      codexFullAlerted = false;
    }
  }

  // 刷新成功后按「刚刚刷新 / N分钟前更新」展示，每分钟更新一次
  function renderRefreshElapsed() {
    if (lastRefreshAt === null) return;
    const minutes = Math.floor((Date.now() - lastRefreshAt) / 60000);
    updated.textContent = minutes === 0 ? '刚刚刷新' : `${minutes}分钟前更新`;
  }

  // 按窗口剩余额度着色：<=60% 橙色，<=30% 红色；非百分比内容（余额、状态文案）原样展示
  function renderQuotaValue(node, value) {
    node.innerHTML = value.split(' / ').map((part) => {
      const match = part.match(/(\d+)%/);
      if (!match) return part;
      const pct = Number(match[1]);
      const cls = pct <= 30 ? 'pct-red' : pct <= 60 ? 'pct-orange' : '';
      return cls ? `<span class="${cls}">${part}</span>` : part;
    }).join(' / ');
  }

  async function refreshQuotas() {
    if (refreshInProgress) return;
    refreshInProgress = true;
    updated.textContent = '刷新中…';
    try {
      const lines = await invoke('get_quotas');
      lines.forEach(({ provider, value, plan }) => {
        const node = document.querySelector(`#${ids[provider]}`);
        if (node) renderQuotaValue(node, value);
        const planNode = document.querySelector(`#${planIds[provider]}`);
        if (planNode) planNode.textContent = plan || '';
      });
      lastRefreshAt = Date.now();
      renderRefreshElapsed();
      await maybeAlertCodexFull(lines);
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

  // 启动及每 6 小时检查一次 GitHub 最新 Release，有新版本则后台下载安装并重启
  async function checkForUpdates() {
    try {
      const update = await checkUpdate();
      if (!update) return;
      updated.textContent = `更新 v${update.version} 中…`;
      await update.downloadAndInstall();
      await relaunch();
    } catch (error) {
      console.error('检查更新失败', error);
    }
  }

  refreshQuotas().catch(console.error);
  window.setInterval(() => { refreshQuotas().catch(console.error); }, 5 * 60 * 1000);
  window.setInterval(renderRefreshElapsed, 60 * 1000);
  checkForUpdates().catch(console.error);
  window.setInterval(() => { checkForUpdates().catch(console.error); }, 6 * 60 * 60 * 1000);
}
