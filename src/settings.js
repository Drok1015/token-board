import { invoke } from '@tauri-apps/api/core';

const DEFAULT_SETTINGS = {
  autoHide: false,
  hideDelaySeconds: 10,
  showPlans: true,
  visibleProviders: ['CODEX', 'KIMI', 'GLM', 'DEEPSEEK'],
  glmApiKey: '',
  deepseekApiKey: '',
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
          <p>控制看板展开后的自动收起行为</p>
        </div>
      </header>
      <form id="settings-form">
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
        <div class="setting-row setting-providers-header">
          <span>
            <strong>显示的供应商</strong>
            <small>勾选后展示在看板中，至少保留一个</small>
          </span>
        </div>
        <div id="provider-list"></div>
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
  const showPlans = document.querySelector('#show-plans');
  const message = document.querySelector('#settings-message');
  const saveButton = document.querySelector('[type="submit"][form="settings-form"]');

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

  function syncDelayState() {
    hideDelay.disabled = !autoHide.checked;
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
      message.textContent = '至少选择一个供应商';
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
          visibleProviders,
          glmApiKey: providerRows.find(({ provider }) => provider.keyField === 'glmApiKey').keyInput.value.trim(),
          deepseekApiKey: providerRows.find(({ provider }) => provider.keyField === 'deepseekApiKey').keyInput.value.trim(),
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
      autoHide.checked = Boolean(settings.autoHide);
      hideDelay.value = String(settings.hideDelaySeconds);
      showPlans.checked = Boolean(settings.showPlans);
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
