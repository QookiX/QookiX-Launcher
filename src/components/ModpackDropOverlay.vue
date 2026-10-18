<script setup lang="ts">
/**
 * 全局整合包拖拽识别。
 *
 * Tauri 原生拖放（`dragDropEnabled` 未显式关闭，Tauri v2 默认开启）会在 OS 层
 * 拦截文件拖放，webview 收不到 HTML5 的 `dragover`/`drop`，因此这里直接监听
 * `getCurrentWebview().onDragDropEvent`。
 *
 * 行为：拖入的文件若是整合包（.mrpack / .zip），显示全屏投放提示；松手后弹确认框，
 * 确认即调用 `import_modpack` 创建实例并跳转到该实例页。非整合包文件被忽略。
 */
import { onBeforeUnmount, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { useDialog, useMessage } from "naive-ui";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useI18n } from "vue-i18n";
import { api } from "../api";
import { useInstancesStore } from "../stores/instances";
import { instanceDetailActive } from "../composables/instanceDetailDrag";
import { IconPackage } from "./icons";

const MODPACK_RE = /\.(mrpack|zip)$/i;
const ZIP_RE = /\.zip$/i;

const router = useRouter();
const dialog = useDialog();
const message = useMessage();
const instances = useInstancesStore();
const { t } = useI18n();

// 是否正有整合包文件被拖到窗口上方、以及它的文件名（用于全屏提示）
const dragging = ref(false);
const dragName = ref("");
const importing = ref(false);

let unlisten: (() => void) | null = null;

function basename(p: string): string {
  return p.split(/[\\/]/).filter(Boolean).pop() ?? p;
}

function findModpack(paths: string[]): string | null {
  // 实例详情页激活时让出 .zip（交给实例详情页当光影/材质包），只认 .mrpack
  return paths.find((p) => MODPACK_RE.test(p) && (!instanceDetailActive.value || !ZIP_RE.test(p))) ?? null;
}

async function doImport(file: string) {
  if (importing.value) return;
  importing.value = true;
  try {
    // 命令只负责建实例（快速返回）；内容下载与游戏本体安装由后端后台完成，
    // 进度通过 install://progress 事件进下载中心。
    const inst = await api.importModpack(file);
    message.success(t("createInstance.modpackImported", { name: inst.name }));
    await instances.refresh();
    router.push(`/instance/${inst.id}`);
  } catch (e) {
    message.error(String(e));
  } finally {
    importing.value = false;
  }
}

function onDrop(file: string) {
  if (importing.value) return;
  const name = basename(file);
  dialog.info({
    title: t("drop.confirmTitle"),
    content: t("drop.confirmContent", { name }),
    positiveText: t("drop.confirmImport"),
    negativeText: t("common.cancel"),
    // 不返回 promise：立即关闭确认框，导入进度由全局 loading 消息承载
    onPositiveClick: () => { void doImport(file); },
  });
}

onMounted(async () => {
  try {
    unlisten = await getCurrentWebview().onDragDropEvent((event) => {
      const p = event.payload;
      if (p.type === "enter") {
        const file = findModpack(p.paths);
        if (file) {
          dragName.value = basename(file);
          dragging.value = true;
        }
      } else if (p.type === "drop") {
        dragging.value = false;
        const file = findModpack(p.paths);
        if (file) onDrop(file);
      } else if (p.type === "leave") {
        dragging.value = false;
      }
      // `over` 无需处理：保持 enter 时确定的状态即可
    });
  } catch {
    /* 拖放监听不可用时不影响其他功能 */
  }
});

onBeforeUnmount(() => {
  unlisten?.();
  unlisten = null;
});
</script>

<template>
  <Transition name="drop-fade">
    <div v-if="dragging" class="drop-overlay">
      <div class="drop-card">
        <div class="drop-icon"><IconPackage /></div>
        <div class="drop-title">{{ t("drop.overlayTitle") }}</div>
        <div class="drop-name">{{ dragName }}</div>
        <div class="drop-desc">{{ t("drop.overlayDesc", { name: dragName }) }}</div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.drop-overlay {
  position: fixed;
  inset: 0;
  z-index: 1400;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 40px;
  /* 纯视觉提示：原生拖放期间不应拦截任何指针事件 */
  pointer-events: none;
  background: rgba(0, 0, 0, 0.55);
  backdrop-filter: blur(3px);
  -webkit-backdrop-filter: blur(3px);
  box-sizing: border-box;
}
.drop-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 40px 56px;
  max-width: 520px;
  border-radius: 18px;
  border: 2px dashed var(--accent);
  background: color-mix(in srgb, var(--feat-solid) 92%, transparent);
  box-shadow: 0 24px 70px rgba(0, 0, 0, 0.45);
  color: var(--text-1);
  text-align: center;
}
.drop-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 64px;
  height: 64px;
  border-radius: 50%;
  background: var(--accent-soft);
  color: var(--accent);
  font-size: 32px;
}
.drop-title {
  font-size: 18px;
  font-weight: 700;
}
.drop-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--accent);
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.drop-desc {
  font-size: 12px;
  color: var(--text-3);
}
.drop-fade-enter-active,
.drop-fade-leave-active {
  transition: opacity 0.15s ease;
}
.drop-fade-enter-from,
.drop-fade-leave-to {
  opacity: 0;
}
</style>
