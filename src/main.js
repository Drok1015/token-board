import { invoke } from '@tauri-apps/api/core';
import './style.css';

const app = document.querySelector('#app');

app.innerHTML = `
  <section class="pet" aria-label="汇兑小猪桌面助手">
    <div class="launcher" id="launcher" aria-live="polite">
      <button class="app-button" data-app="huide" aria-label="打开汇兑">
        <img src="/huide-icon.png" alt="" />
      </button>
      <button class="app-button" data-app="renren" aria-label="打开人人视频">
        <img src="/renren-video-icon.png" alt="" />
      </button>
      <button class="app-button" data-app="parallels" aria-label="打开 Parallels Desktop">
        <img src="/parallels-desktop-icon.png" alt="" />
      </button>
    </div>
    <button class="mascot" id="mascot" aria-label="显示快捷功能" data-tauri-drag-region>
      <img src="/pig-mascot-cropped.png" alt="原创粉色小猪桌面宠物" draggable="false" data-tauri-drag-region />
    </button>
  </section>`;

const launcher = document.querySelector('#launcher');
const mascot = document.querySelector('#mascot');
const appButtons = document.querySelectorAll('.app-button');
let pressPoint = null;
let dragging = false;
let menuVisible = false;

function setLauncherVisible(visible) {
  if (visible === menuVisible) return;
  menuVisible = visible;
  launcher.classList.toggle('visible', visible);
}

mascot.addEventListener('pointerdown', (event) => {
  pressPoint = { x: event.clientX, y: event.clientY };
  dragging = false;
});

mascot.addEventListener('pointermove', (event) => {
  if (!pressPoint || dragging) return;
  if (Math.hypot(event.clientX - pressPoint.x, event.clientY - pressPoint.y) >= 5) dragging = true;
});

mascot.addEventListener('pointerup', () => {
  pressPoint = null;
});

mascot.addEventListener('click', () => {
  if (dragging) return;
  setLauncherVisible(!menuVisible);
});

appButtons.forEach((button) => button.addEventListener('click', async () => {
  button.classList.add('launching');
  try {
    await invoke('open_app', { app: button.dataset.app });
    setLauncherVisible(false);
  } catch (error) {
    console.error(error);
  } finally {
    setTimeout(() => button.classList.remove('launching'), 300);
  }
}));
