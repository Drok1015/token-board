import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
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
const BOARD_VERTICAL_MARGIN = 24; // 窗口比屏幕体高出的上下透明边距（阴影留位）
const EDGE_TAB_WIDTH = 24;
const EDGE_TAB_HEIGHT = 72;
const SNAP_DISTANCE = 16;
const FADE_DURATION = 100;
// 官方重置追踪站的公开 API：返回 @thsottiaux 宣布的重置事件，按时间倒序，时间为 UTC
const CODEX_RESETS_API = 'https://codex-resets.com/api/resets';
const CODEX_RESETS_TIMEOUT = 15 * 1000;
const AUTO_UPDATE_INTERVAL = 2 * 60 * 60 * 1000;
const QUOTA_REFRESH_INTERVAL = 5 * 60 * 1000;
const MINUTE_TICK_INTERVAL = 60 * 1000;
const DEFAULT_SETTINGS = {
  autoHide: false,
  hideDelaySeconds: 10,
  showPlans: true,
  visibleProviders: ['CODEX', 'KIMI', 'GLM', 'DEEPSEEK'],
  autoUpdate: true,
  codexAlert: true,
  showBoard: true,
  showResets: true,
};

export function mountBoard() {
  const app = document.querySelector('#app');
  const appWindow = getCurrentWindow();

  app.innerHTML = `
    <main class="app-shell" id="app-shell">
      <section class="board" aria-label="Token 看板" data-tauri-drag-region>
        <div class="screen" aria-live="polite" data-tauri-drag-region>
          <div class="screen-title">TOKEN 看板 <span class="signal">●</span><span class="updated" id="updated">自动刷新</span></div>
          <div class="quota-row" data-provider="CODEX"><b>CODEX</b><span class="plan-badge" id="codex-plan"></span><span class="quota-value" id="codex">读取中…</span><span class="reset-badge" id="codex-reset" aria-label="额度重置时间">⏳</span></div>
          <div class="quota-row" data-provider="KIMI"><b>KIMI</b><span class="plan-badge" id="kimi-plan"></span><span class="quota-value" id="kimi">读取中…</span><span class="reset-badge" id="kimi-reset" aria-label="额度重置时间">⏳</span></div>
          <div class="quota-row" data-provider="GLM"><b>GLM<span class="peak-badge" id="glm-peak">(高)</span></b><span class="plan-badge" id="glm-plan"></span><span class="quota-value" id="glm">读取中…</span><span class="reset-badge" id="glm-reset" aria-label="额度重置时间">⏳</span></div>
          <div class="quota-row" data-provider="DEEPSEEK"><b>DEEPSEEK<span class="peak-badge" id="deepseek-peak">(高)</span></b><span class="plan-badge" id="deepseek-plan"></span><span class="quota-value" id="deepseek">读取中…</span><span class="reset-badge" id="deepseek-reset" aria-label="额度重置时间">⏳</span></div>
          <div class="screen-footer"><span>5分钟刷新一次，可右键手动刷新</span><span class="version" id="version"></span></div>
        </div>
      </section>
      <button class="edge-tab" id="edge-tab" type="button" aria-label="展开 Token 看板" title="点击展开；上下拖动调整位置">›</button>
      <div class="reset-tip" id="reset-tip" hidden></div>
    </main>`;

  const ids = { CODEX: 'codex', KIMI: 'kimi', GLM: 'glm', DEEPSEEK: 'deepseek' };
  const planIds = { CODEX: 'codex-plan', KIMI: 'kimi-plan', GLM: 'glm-plan', DEEPSEEK: 'deepseek-plan' };
  const resetIds = { CODEX: 'codex-reset', KIMI: 'kimi-reset', GLM: 'glm-reset', DEEPSEEK: 'deepseek-reset' };
  const shell = document.querySelector('#app-shell');
  const edgeTab = document.querySelector('#edge-tab');
  const updated = document.querySelector('#updated');
  getVersion().then((version) => {
    document.querySelector('#version').textContent = `v${version}`;
  }).catch(console.error);
  let edgeState = null;
  let transitioning = false;
  let edgePointer = null;
  let ignoreTabClick = false;
  let settings = { ...DEFAULT_SETTINGS };
  let autoHideTimer = null;
  let updateTimer = null;
  let updateCheckInProgress = false;
  let refreshInProgress = false;
  let codexLastResetAt = null; // 最近一次已知的重置事件时间（ISO 字符串），用于识别新重置
  let lastRefreshAt = null;
  let resetsByProvider = {}; // 各供应商最近一次刷新拿到的窗口重置时间，供重置图标悬浮展示
  let lastTrayLines = null; // 最近一次推给托盘的数据（不含 peak），高峰期切换时补发
  let prevPeakKey = ''; // 上次各供应商的高峰期状态，变更时才重推托盘
  let boardHeight = BOARD_HEIGHT; // 最近一次的实测窗口高度；贴边隐藏时 DOM 不可见，用缓存值
  let moveHandling = false;
  let pendingMovePosition = null;

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

  // 高度不写死：实测屏幕体高度加透明边距；行数增减、字号或样式变化都会自动跟随
  function currentBoardHeight() {
    const measured = document.querySelector('.screen').getBoundingClientRect().height;
    if (measured > 0) boardHeight = Math.ceil(measured) + BOARD_VERTICAL_MARGIN;
    return boardHeight;
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
    // 显示位置：面板关闭时隐藏悬浮窗（任务栏仍可通过托盘菜单打开设置）
    if (settings.showBoard) {
      await appWindow.show();
    } else {
      await appWindow.hide();
    }
    await resizeBoardForSettings();
    scheduleAutoHide();
    scheduleAutoUpdate();
    renderResetBadges();
  }

  // 自动更新默认开启（设置可关）：开启后立即检查一次并每 2 小时轮询；
  // 关闭时清除轮询（右键菜单手动检查不受影响）。检查设 30 秒超时，GitHub 抽风时不挂死，等下轮重试
  function scheduleAutoUpdate() {
    if (updateTimer !== null) {
      window.clearInterval(updateTimer);
      updateTimer = null;
    }
    if (!settings.autoUpdate) return;
    checkForUpdates().catch(console.error);
    updateTimer = window.setInterval(() => { checkForUpdates().catch(console.error); }, AUTO_UPDATE_INTERVAL);
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

  // 拖动窗口时 onMoved 可能在一次 IPC 尚未完成前连续触发；只处理最新坐标，避免堆积
  // currentMonitor / outerSize / setPosition 调用造成主线程和 WebView 往返拥堵。
  async function handleWindowMoved(physicalPosition) {
    pendingMovePosition = physicalPosition;
    if (moveHandling) return;
    moveHandling = true;
    try {
      while (pendingMovePosition !== null) {
        const latest = pendingMovePosition;
        pendingMovePosition = null;
        if (edgeState) await pinEdgeTab(latest);
        else await checkSnap(latest);
      }
    } finally {
      moveHandling = false;
    }
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

  // CODEX 重置改为查询 codex-resets.com 的公开 API（追踪 @thsottiaux 官宣重置的推文），
  // 随额度刷新每 5 分钟查询一次：取最新一条事件的 announced_at（UTC），比之前记录的更新
  // 即视为发生新重置并弹系统提醒。首次查询静默建立基线（不为历史重置补弹提醒），同一事件
  // 不会重复提醒。请求失败时保留基线等下个周期重试；可在设置中关闭，关闭期间不跟踪，
  // 重新开启后从下一次查询重新建立基线
  async function maybeAlertCodexReset() {
    if (!settings.codexAlert) {
      codexLastResetAt = null;
      return;
    }
    const controller = new AbortController();
    const timeout = window.setTimeout(() => controller.abort(), CODEX_RESETS_TIMEOUT);
    try {
      const response = await fetch(CODEX_RESETS_API, {
        headers: { accept: 'application/json' },
        signal: controller.signal,
      });
      if (!response.ok) return;
      const latest = (await response.json()).events?.[0]?.announced_at;
      if (!latest) return;
      // ISO 时间戳可直接按字符串比较先后
      if (codexLastResetAt !== null && latest > codexLastResetAt) {
        await invoke('notify_codex_full');
      }
      codexLastResetAt = latest;
    } catch (error) {
      console.error('查询 CODEX 重置状态失败', error);
    } finally {
      window.clearTimeout(timeout);
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

  // 重置图标：悬浮 0.5 秒后显示自绘提示气泡，列出各额度窗口（如 CODEX / KIMI / GLM 的
  // 5h、7d）的重置时间；无重置数据或设置关闭时图标隐藏。不用原生 title 是因为其延迟不可控
  const RESET_TIP_DELAY = 500;
  const resetTip = document.querySelector('#reset-tip');
  let resetTipTimer = null;

  function formatResetTime(ms) {
    const date = new Date(ms);
    if (Number.isNaN(date.getTime())) return '未知';
    const time = date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', hour12: false });
    return date.toDateString() === new Date().toDateString()
      ? time
      : `${date.getMonth() + 1}月${date.getDate()}日 ${time}`;
  }

  function hideResetTip() {
    if (resetTipTimer !== null) {
      window.clearTimeout(resetTipTimer);
      resetTipTimer = null;
    }
    resetTip.hidden = true;
  }

  const isWorkday = (date) => date.getDay() !== 0 && date.getDay() !== 6;

  // DeepSeek 高峰期：周一到周五 9:00–12:00、14:00–18:00（本机时区）
  function isDeepSeekPeak(date = new Date()) {
    if (!isWorkday(date)) return false;
    const minutes = date.getHours() * 60 + date.getMinutes();
    return (minutes >= 9 * 60 && minutes < 12 * 60) || (minutes >= 14 * 60 && minutes < 18 * 60);
  }

  // GLM 高峰期：周一到周五 14:00–18:00（智谱官方规则，按北京时间 UTC+8 判定，此期间积分消耗加倍）；
  // 先把时刻换算到北京时间再取星期与小时，不受本机时区影响
  function isGlmPeak(date = new Date()) {
    const beijing = new Date(date.getTime() + (date.getTimezoneOffset() + 480) * 60000);
    return isWorkday(beijing) && beijing.getHours() >= 14 && beijing.getHours() < 18;
  }

  const PEAK_CHECKS = { GLM: isGlmPeak, DEEPSEEK: isDeepSeekPeak };
  const PEAK_BADGE_IDS = { GLM: 'glm-peak', DEEPSEEK: 'deepseek-peak' };
  const PEAK_TIPS = {
    GLM: '高峰期为周一到周五 14:00–18:00（北京时间），期间积分消耗加倍',
    DEEPSEEK: '高峰期为周一到周五 9:00–12:00、14:00–18:00，期间名称后显示(高)',
  };

  // 每分钟重估一次高峰期(高)标识显隐（额度本身 5 分钟才刷新一次）；
  // 高峰状态翻转时同步重推托盘，让菜单栏的供应商名也实时带上 (高)
  function applyPeakBadges(now = new Date()) {
    const peakByProvider = {};
    Object.entries(PEAK_BADGE_IDS).forEach(([provider, id]) => {
      peakByProvider[provider] = PEAK_CHECKS[provider](now);
      const badge = document.querySelector(`#${id}`);
      if (!badge) return;
      const valueNode = document.querySelector(`#${ids[provider]}`);
      // 状态文案（读取中/失败/未配置）不显示高峰期标识
      const hasValue = Boolean(valueNode) && !['读取中…', '读取失败', '未配置'].includes(valueNode.textContent);
      badge.classList.toggle('visible', hasValue && peakByProvider[provider]);
    });
    const peakKey = Object.entries(peakByProvider).map(([name, inPeak]) => `${name}:${inPeak ? 1 : 0}`).join(',');
    if (peakKey !== prevPeakKey) {
      prevPeakKey = peakKey;
      pushTrayLines();
    }
  }

  // 把最近一次额度数据附带当前高峰期状态推给托盘渲染；额度尚未拉到时不推
  function pushTrayLines() {
    if (!lastTrayLines || refreshInProgress) return;
    invoke('update_tray', { lines: lastTrayLines.map(({ provider, value }) => ({
      provider,
      value,
      peak: Boolean(PEAK_CHECKS[provider] && PEAK_CHECKS[provider]()),
    })) }).catch(console.error);
  }

  function showResetTip(node) {
    const provider = node.dataset.provider;
    let text;
    if (provider === 'DEEPSEEK') {
      // DeepSeek 没有窗口化额度，重置图标悬浮展示固定的高峰期说明
      text = PEAK_TIPS.DEEPSEEK;
    } else {
      const resets = resetsByProvider[provider] || [];
      if (resets.length === 0) {
        text = PEAK_TIPS[provider];
      } else {
        text = resets
          .map(({ label, resetsAtMs }) => `${label} 重置：${formatResetTime(resetsAtMs)}`)
          .join('\n');
        if (PEAK_TIPS[provider]) text += `\n${PEAK_TIPS[provider]}`;
      }
    }
    resetTip.textContent = text;
    resetTip.hidden = false;
    // 气泡跟随图标位置，优先显示在下方；贴近窗口底部时翻到上方，并夹在窗口可视范围内
    const badgeRect = node.getBoundingClientRect();
    const tipRect = resetTip.getBoundingClientRect();
    const left = clamp(badgeRect.left + badgeRect.width / 2 - tipRect.width / 2, 8, window.innerWidth - tipRect.width - 8);
    let top = badgeRect.bottom + 6;
    if (top + tipRect.height > window.innerHeight - 4) top = badgeRect.top - tipRect.height - 6;
    resetTip.style.left = `${Math.round(left)}px`;
    resetTip.style.top = `${Math.round(Math.max(top, 4))}px`;
  }

  function renderResetBadges() {
    Object.entries(resetIds).forEach(([provider, id]) => {
      const node = document.querySelector(`#${id}`);
      if (!node) return;
      node.dataset.provider = provider;
      // DEEPSEEK 重置图标是固定的高峰期说明，不受额度数据影响
      const hasTip = settings.showResets && (provider === 'DEEPSEEK' || (resetsByProvider[provider] || []).length > 0);
      node.classList.toggle('visible', hasTip);
      if (!hasTip) hideResetTip();
    });
  }

  Object.entries(resetIds).forEach(([provider, id]) => {
    const node = document.querySelector(`#${id}`);
    if (!node) return;
    node.dataset.provider = provider;
    node.addEventListener('mouseenter', () => {
      hideResetTip();
      resetTipTimer = window.setTimeout(() => {
        resetTipTimer = null;
        showResetTip(node);
      }, RESET_TIP_DELAY);
    });
    node.addEventListener('mouseleave', hideResetTip);
  });

  async function refreshQuotas() {
    if (refreshInProgress) return;
    refreshInProgress = true;
    updated.textContent = '刷新中…';
    try {
      const lines = await invoke('get_quotas');
      lines.forEach(({ provider, value, plan, resets }) => {
        const node = document.querySelector(`#${ids[provider]}`);
        if (node) renderQuotaValue(node, value);
        const planNode = document.querySelector(`#${planIds[provider]}`);
        if (planNode) planNode.textContent = plan || '';
        resetsByProvider[provider] = Array.isArray(resets) ? resets : [];
      });
      renderResetBadges();
      applyPeakBadges();
      lastRefreshAt = Date.now();
      renderRefreshElapsed();
      await maybeAlertCodexReset();
      lastTrayLines = lines.map(({ provider, value }) => ({ provider, value }));
    } catch (error) {
      console.error(error);
      Object.values(ids).forEach((id) => {
        document.querySelector(`#${id}`).textContent = '读取失败';
      });
      updated.textContent = '刷新失败';
    } finally {
      refreshInProgress = false;
    }
    // 注意：必须在 finally 复位 refreshInProgress 之后推送，否则会被自身防守拦截
    pushTrayLines();
  }

  async function createContextMenu() {
    const refreshItem = await MenuItem.new({
      id: 'refresh',
      text: '刷新',
      action: () => { refreshQuotas().catch(console.error); },
    });
    const updateItem = await MenuItem.new({
      id: 'update',
      text: '检查更新',
      action: () => { checkForUpdates(true).catch(console.error); },
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
    return Menu.new({ items: [refreshItem, updateItem, settingsItem, closeItem] });
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
    handleWindowMoved(payload).catch(console.error);
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

  // 托盘菜单的「检查更新」经 Rust 转发到这里执行
  listen('tray-check-updates', () => {
    checkForUpdates(true).catch(console.error);
  }).catch(console.error);

  // 托盘菜单的「刷新」
  listen('tray-refresh', () => {
    refreshQuotas().catch(console.error);
  }).catch(console.error);

  invoke('get_settings')
    .then((value) => applySettings(value))
    .catch(console.error);

  // 检查 GitHub 最新 Release，有新版本则后台下载安装并重启；定时调用由 scheduleAutoUpdate 控制，右键菜单可随时手动触发
  // 双保险：未开启自动更新时，非手动触发一律直接返回
  async function checkForUpdates(manual = false) {
    if (!manual && !settings.autoUpdate) return;
    if (updateCheckInProgress) return;
    updateCheckInProgress = true;
    try {
      let update = null;
      try {
        update = await checkUpdate({ timeout: 30 * 1000 });
      } catch (error) {
        console.error('检查更新失败', error);
        if (manual) updated.textContent = '检查更新失败';
        return;
      }
      if (!update) {
        if (manual) updated.textContent = '已是最新版本';
        return;
      }
      // 下载安装失败（常见于安装包未解除 quarantine 权限限制）时弹窗引导用户手动处理
      try {
        updated.textContent = `更新 v${update.version} 中…`;
        await update.downloadAndInstall();
        await relaunch();
      } catch (error) {
        console.error('安装更新失败', error);
        updated.textContent = '更新失败';
        invoke('notify_update_failed').catch(console.error);
      }
    } finally {
      updateCheckInProgress = false;
    }
  }

  refreshQuotas().catch(console.error);
  window.setInterval(() => { refreshQuotas().catch(console.error); }, QUOTA_REFRESH_INTERVAL);
  // 合并分钟级 UI 更新与高峰期检查，减少后台定时器唤醒次数。
  window.setInterval(() => {
    renderRefreshElapsed();
    applyPeakBadges();
  }, MINUTE_TICK_INTERVAL);
}
