import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import router from "./router";
import i18n from "./i18n";
import { installMessageCopy } from "./composables/notify";
import { installDevToolsHotkey } from "./composables/useDevFlags";
import "./styles.css";

// 顶部 message 提示：点一下即可复制内容（错误详情这类长文本尤其有用）
installMessageCopy();
// F12 等 DevTools 快捷键默认拦下，只有开发者页开关放行时才打开
installDevToolsHotkey();

// 桌面启动器不需要浏览器默认右键菜单。
// 但输入框必须放行：安装路径、服务器地址、下载链接这些内容，右键粘贴是最常用的一条路径，
// 全拦掉等于把「粘贴」变成只能靠 Ctrl+V。
const isEditableTarget = (el: EventTarget | null): boolean =>
  el instanceof HTMLElement && (el.isContentEditable || !!el.closest("input, textarea, [contenteditable='true']"));

window.addEventListener("contextmenu", (e) => {
  if (!isEditableTarget(e.target)) e.preventDefault();
});


// 懒加载页面（路由 chunk）拉取失败时 Vite 会派发 preloadError：
// dev 下多见于长时间 HMR 后模块失效，正式版多见于发版后旧 chunk 被清掉。
// 不处理的话表现就是"点页面/点卡片没反应"（路由既不切换也不报错），
// 所以整体重载一次把它救回来；10 秒内只重载一次，避免一直失败时无限刷新。
window.addEventListener("vite:preloadError", () => {
  const key = "qookix:preload-reload-at";
  const last = Number(sessionStorage.getItem(key) ?? 0);
  if (Date.now() - last < 10_000) return;
  sessionStorage.setItem(key, String(Date.now()));
  window.location.reload();
});

const app = createApp(App);
app.use(createPinia());
app.use(router);
app.use(i18n);
app.mount("#app");
