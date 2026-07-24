import { invoke } from '@tauri-apps/api/core';
import './style.css';

const app = document.querySelector('#app');

app.innerHTML = `
  <section class="board" aria-label="额度脉搏看板" data-tauri-drag-region>
    <div class="screen" aria-live="polite" data-tauri-drag-region>
      <div class="screen-title">额度脉搏 <span class="updated">自动刷新</span><span class="signal">●</span></div>
      <div class="quota-row"><b>CODEX</b><span id="codex">读取中…</span></div>
      <div class="quota-row"><b>KIMI</b><span id="kimi">读取中…</span></div>
      <div class="quota-row"><b>GLM</b><span id="glm">读取中…</span></div>
      <div class="quota-row"><b>DEEPSEEK</b><span id="deepseek">读取中…</span></div>
    </div>
  </section>`;

const ids = { CODEX: 'codex', KIMI: 'kimi', GLM: 'glm', DEEPSEEK: 'deepseek' };

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
