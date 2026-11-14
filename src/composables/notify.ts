/**
 * 全局消息 API 持有器。
 *
 * naive-ui 的 useMessage() 只能在组件 setup 上下文中调用；
 * 在异步回调（如 deep-link 处理、全局事件）里调用会直接抛异常。
 * App.vue 在 setup 中调用 setMessageApi() 存下实例，
 * 任何非组件代码通过 notifySuccess/notifyError 使用。
 */

import { defineComponent, h } from "vue";
import { useMessage, type MessageApi } from "naive-ui";
import { log as devLog, error as devError, warn as devWarn } from "../utils/logger";
import { copyText } from "../utils/clipboard";
import i18n from "../i18n";

let holder: MessageApi | null = null;

export function setMessageApi(api: MessageApi) {
  holder = api;
}

/** 放在 n-message-provider 内部，把 MessageApi 存入模块级持有器 */
export const MessageBridge = defineComponent({
  name: "MessageBridge",
  setup() {
    setMessageApi(useMessage());
    return () => h("span", { style: "display:none" });
  },
});

export function notifySuccess(content: string) {
  try {
    holder?.success(content);
  } catch {
    devLog("[notify]", content);
  }
}

export function notifyError(content: string) {
  try {
    holder?.error(content);
  } catch {
    devError("[notify]", content);
  }
}

export function notifyWarning(content: string) {
  try {
    holder?.warning(content);
  } catch {
    devWarn("[notify]", content);
  }
}

/* -------------------------------------------------------------------------- */
/* 点击 message 提示条复制内容                                                  */
/* -------------------------------------------------------------------------- */

let messageCopyInstalled = false;

/**
 * naive-ui 的 message 是 teleport 到 body 的（`.n-message-container--top`），
 * 页面组件里够不到，所以在 document 上做一次事件委托，统一覆盖全项目的
 * `message.success/error/warning/info/loading` —— 包括错误详情这类长文本。
 *
 * 文本只取 `.n-message__content`，不会把图标或关闭按钮混进剪贴板。
 * 复制结果由 `data-qx-copy` 驱动，样式见 `src/styles.css` 的第三方覆写区。
 */
export function installMessageCopy() {
  if (messageCopyInstalled) return;
  messageCopyInstalled = true;

  // 悬停时用原生 tooltip 说明「可以点」，不改 message 自身结构
  document.addEventListener("pointerover", (event) => {
    if (!(event.target instanceof Element)) return;
    const msg = event.target.closest<HTMLElement>(".n-message-container .n-message");
    if (msg && !msg.title) msg.title = i18n.global.t("common.clickToCopy");
  });

  document.addEventListener("click", (event) => {
    if (!(event.target instanceof Element)) return;
    // 关闭按钮与提示内的链接/按钮保留自己的行为；用户手动选了文本也尊重选区
    if (event.target.closest(".n-message__close, a, button, [role='button']")) return;
    if (window.getSelection()?.toString().trim()) return;

    const msg = event.target.closest<HTMLElement>(".n-message-container .n-message");
    if (!msg) return;
    const content = msg.querySelector<HTMLElement>(".n-message__content");
    const text = (content?.innerText ?? msg.innerText ?? "").trim();
    if (!text) return;

    void copyText(text).then((ok) => {
      if (!msg.isConnected) return; // 提示可能已经自动消失
      // 不在正文后面插「已复制」文字：改成整条提示高亮闪一下 + 悬停文案变化，
      // 这样用户选中文本或再点一次时不会把反馈文字混进剪贴板。
      // 先移除再强制重排，连点时高亮动画才会重新播放。
      delete msg.dataset.qxCopy;
      void msg.offsetWidth;
      msg.dataset.qxCopy = ok ? "ok" : "fail";
      msg.title = i18n.global.t(ok ? "common.copied" : "common.copyFailed");
      const prev = Number(msg.dataset.qxCopyTimer ?? 0);
      if (prev) window.clearTimeout(prev);
      msg.dataset.qxCopyTimer = String(
        window.setTimeout(() => {
          delete msg.dataset.qxCopy;
          delete msg.dataset.qxCopyTimer;
          msg.title = i18n.global.t("common.clickToCopy");
        }, 1400),
      );
    });
  });
}
