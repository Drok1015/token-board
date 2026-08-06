import { invoke } from '@tauri-apps/api/core';

const DEFAULT_SETTINGS = {
  autoHide: false,
  hideDelaySeconds: 10,
  showPlans: true,
  visibleProviders: ['CODEX', 'KIMI', 'GLM', 'DEEPSEEK'],
  glmApiKey: '',
  deepseekApiKey: '',
  autoUpdate: true,
  trayProvider: 'CODEX',
  codexAlert: true,
  showBoard: true,
  showTray: true,
};

const PROVIDERS = [
  { name: 'CODEX', label: 'CODEX', hint: '读取本机 codex 登录状态' },
  { name: 'KIMI', label: 'KIMI', hint: '读取本机 Kimi Code 凭据' },
  { name: 'GLM', label: 'GLM', hint: '默认从 cc-switch 读取', keyField: 'glmApiKey', keyId: 'glm-api-key' },
  { name: 'DEEPSEEK', label: 'DeepSeek', hint: '默认从 cc-switch 读取', keyField: 'deepseekApiKey', keyId: 'deepseek-api-key' },
];

export function mountSettings() {
  document.body.classList.add('settings-mode');
  const app = document.querySelector('#app');

  app.innerHTML = `
    <main class="settings-panel">
      <header class="settings-header">
        <div class="settings-icon" aria-hidden="true">◷</div>
        <div>
          <h1>Token 看板设置</h1>
          <p>看板显示、提醒与供应商的个性化设置</p>
        </div>
      </header>
      <nav class="settings-tabs" aria-label="设置分类">
        <button type="button" class="settings-tab active" data-tab="display">显示</button>
        <button type="button" class="settings-tab" data-tab="notify">通知</button>
        <button type="button" class="settings-tab" data-tab="providers">供应商</button>
      </nav>
      <form id="settings-form">
        <div class="tab-panel" data-panel="display">
          <label class="setting-row setting-toggle">
            <span>
              <strong>自动隐藏看板</strong>
              <small>展示一段时间后收起到最近的屏幕边缘</small>
            </span>
            <input id="auto-hide" type="checkbox">
          </label>
          <label class="setting-row setting-delay" for="hide-delay">
            <span>
              <strong>展示时长</strong>
              <small>每次点击边缘箭头后重新计时</small>
            </span>
            <span class="seconds-input">
              <input id="hide-delay" type="number" min="1" max="3600" step="1" value="10">
              <span>秒</span>
            </span>
          </label>
          <label class="setting-row setting-toggle">
            <span>
              <strong>显示订阅套餐</strong>
              <small>在供应商名称后显示 Plus、Allegretto 等套餐标签</small>
            </span>
            <input id="show-plans" type="checkbox">
          </label>
          <div class="setting-group">
            <div class="setting-row setting-providers-header">
              <span>
                <strong>显示位置</strong>
                <small>至少保留一个；任务栏指屏幕顶部菜单栏状态区</small>
              </span>
            </div>
            <label class="setting-row setting-toggle provider-row">
              <span>
                <strong>面板</strong>
                <small>桌面悬浮的 Token 看板窗口</small>
              </span>
              <input id="show-board" type="checkbox">
            </label>
            <label class="setting-row setting-toggle provider-row">
              <span>
                <strong>任务栏</strong>
                <small>菜单栏状态区的彩色额度文字</small>
              </span>
              <input id="show-tray" type="checkbox">
            </label>
          </div>
        </div>
        <div class="tab-panel" data-panel="notify" hidden>
          <label class="setting-row setting-toggle">
            <span>
              <strong>CODEX 重置提醒</strong>
              <small>每 5 分钟查询 codex-resets.com，检测到新重置时弹系统对话框</small>
            </span>
            <input id="codex-alert" type="checkbox">
          </label>
          <label class="setting-row setting-toggle">
            <span>
              <strong>自动更新</strong>
              <small>启动及每 6 小时检查新版本并自动安装，右键菜单可随时手动检查</small>
            </span>
            <input id="auto-update" type="checkbox">
          </label>
        </div>
        <div class="tab-panel" data-panel="providers" hidden>
          <div class="setting-row setting-providers-header">
            <span>
              <strong>显示的供应商</strong>
              <small>勾选后展示在看板中，至少保留一个</small>
            </span>
          </div>
          <div id="provider-list"></div>
        </div>
      </form>
      <footer class="settings-actions">
        <span class="settings-message" id="settings-message" role="status"></span>
        <button class="button-secondary" id="cancel-settings" type="button">取消</button>
        <button class="button-primary" type="submit" form="settings-form">保存</button>
      </footer>
    </main>`;

  const form = document.querySelector('#settings-form');
  const autoHide = document.querySelector('#auto-hide');
  const hideDelay = document.querySelector('#hide-delay');
  const codexAlert = document.querySelector('#codex-alert');
  const showPlans = document.querySelector('#show-plans');
  const autoUpdate = document.querySelector('#auto-update');
  const showBoard = document.querySelector('#show-board');
  const showTray = document.querySelector('#show-tray');
  const delayRow = document.querySelector('.setting-delay');
  const message = document.querySelector('#settings-message');
  const saveButton = document.querySelector('[type="submit"][form="settings-form"]');
  const tabs = document.querySelectorAll('.settings-tab');
  const panels = document.querySelectorAll('.tab-panel');

  function switchTab(name) {
    tabs.forEach((tab) => tab.classList.toggle('active', tab.dataset.tab === name));
    panels.forEach((panel) => { panel.hidden = panel.dataset.panel !== name; });
  }

  tabs.forEach((tab) => {
    tab.addEventListener('click', () => switchTab(tab.dataset.tab));
  });
  // 托盘供应商在托盘菜单里修改，设置页不展示；保存时原样带回，避免被默认值覆盖
  let savedTrayProvider = DEFAULT_SETTINGS.trayProvider;

  // 每个供应商一行；GLM / DeepSeek 勾选时在下方展开 API key 输入框（留空则从 cc-switch 读取）
  const providerList = document.querySelector('#provider-list');
  const providerRows = PROVIDERS.map((provider) => {
    const row = document.createElement('label');
    row.className = 'setting-row setting-toggle provider-row';
    row.innerHTML = `
      <span>
        <strong>${provider.label}</strong>
        <small>${provider.hint}</small>
      </span>
      <input type="checkbox" data-provider="${provider.name}">`;
    providerList.appendChild(row);
    const checkbox = row.querySelector('input');

    let keyInput = null;
    let keyRow = null;
    if (provider.keyField) {
      keyRow = document.createElement('div');
      keyRow.className = 'setting-row apikey-row';
      keyRow.innerHTML = `
        <span>
          <strong>${provider.label} API Key</strong>
          <small>留空则从 cc-switch 读取</small>
        </span>
        <input id="${provider.keyId}" type="password" autocomplete="off" spellcheck="false">`;
      providerList.appendChild(keyRow);
      keyInput = keyRow.querySelector('input');
      checkbox.addEventListener('change', () => {
        keyRow.classList.toggle('visible', checkbox.checked);
      });
    }
    return { provider, checkbox, keyRow, keyInput };
  });

  function syncProviderRows() {
    providerRows.forEach(({ checkbox, keyRow }) => {
      if (keyRow) keyRow.classList.toggle('visible', checkbox.checked);
    });
  }

  // 展示时长只在勾选自动隐藏时显示
  function syncDelayState() {
    hideDelay.disabled = !autoHide.checked;
    delayRow.classList.toggle('visible', autoHide.checked);
  }

  autoHide.addEventListener('change', syncDelayState);
  document.querySelector('#cancel-settings').addEventListener('click', () => {
    invoke('close_settings').catch(console.error);
  });

  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    const seconds = Number.parseInt(hideDelay.value, 10);
    if (!Number.isInteger(seconds) || seconds < 1 || seconds > 3600) {
      message.textContent = '请输入 1–3600 秒';
      hideDelay.focus();
      return;
    }

    const visibleProviders = providerRows
      .filter(({ checkbox }) => checkbox.checked)
      .map(({ provider }) => provider.name);
    if (visibleProviders.length === 0) {
      switchTab('providers');
      message.textContent = '至少选择一个供应商';
      return;
    }
    if (!showBoard.checked && !showTray.checked) {
      switchTab('display');
      message.textContent = '面板和任务栏至少保留一个';
      return;
    }

    saveButton.disabled = true;
    message.textContent = '正在保存…';
    try {
      await invoke('save_settings', {
        settings: {
          autoHide: autoHide.checked,
          hideDelaySeconds: seconds,
          showPlans: showPlans.checked,
          autoUpdate: autoUpdate.checked,
          codexAlert: codexAlert.checked,
          showBoard: showBoard.checked,
          showTray: showTray.checked,
          visibleProviders,
          glmApiKey: providerRows.find(({ provider }) => provider.keyField === 'glmApiKey').keyInput.value.trim(),
          deepseekApiKey: providerRows.find(({ provider }) => provider.keyField === 'deepseekApiKey').keyInput.value.trim(),
          trayProvider: savedTrayProvider,
        },
      });
    } catch (error) {
      console.error(error);
      message.textContent = '保存失败';
      saveButton.disabled = false;
    }
  });

  invoke('get_settings')
    .then((value) => {
      const settings = { ...DEFAULT_SETTINGS, ...value };
      savedTrayProvider = settings.trayProvider;
      autoHide.checked = Boolean(settings.autoHide);
      hideDelay.value = String(settings.hideDelaySeconds);
      showPlans.checked = Boolean(settings.showPlans);
      autoUpdate.checked = Boolean(settings.autoUpdate);
      codexAlert.checked = Boolean(settings.codexAlert);
      showBoard.checked = Boolean(settings.showBoard);
      showTray.checked = Boolean(settings.showTray);
      const visible = Array.isArray(settings.visibleProviders) && settings.visibleProviders.length > 0
        ? settings.visibleProviders
        : DEFAULT_SETTINGS.visibleProviders;
      providerRows.forEach(({ provider, checkbox, keyInput }) => {
        checkbox.checked = visible.includes(provider.name);
        if (keyInput) keyInput.value = settings[provider.keyField] || '';
      });
      syncDelayState();
      syncProviderRows();
    })
    .catch((error) => {
      console.error(error);
      message.textContent = '读取设置失败';
      syncDelayState();
    });
}
