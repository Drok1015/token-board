import { invoke } from '@tauri-apps/api/core';

const DEFAULT_SETTINGS = { autoHide: false, hideDelaySeconds: 10, showPlans: true };

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

    saveButton.disabled = true;
    message.textContent = '正在保存…';
    try {
      await invoke('save_settings', {
        settings: {
          autoHide: autoHide.checked,
          hideDelaySeconds: seconds,
          showPlans: showPlans.checked,
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
      syncDelayState();
    })
    .catch((error) => {
      console.error(error);
      message.textContent = '读取设置失败';
      syncDelayState();
    });
}
