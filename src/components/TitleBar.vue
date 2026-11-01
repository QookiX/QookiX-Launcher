<script setup lang="ts">
import { computed, ref, onMounted, inject } from "vue";
import { useRoute, useRouter } from "vue-router";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  IconClose,
  IconMinus,
  IconSquare,
  IconRestore,
  IconPlus,
  IconTrash,
  IconRefresh,
  IconDownload,
  IconSettings,
  IconMoon,
  IconSun,
  IconStop,
} from "./icons";
import { useTasksStore } from "../stores/tasks";
import { useInstancesStore } from "../stores/instances";
import { useServersStore } from "../stores/servers";
import { useNewsStore } from "../stores/news";
import { useSettingsStore } from "../stores/settings";
import { useMessage } from "naive-ui";
import AccountChip from "./AccountChip.vue";
import { titlebarThemeToggle } from "../composables/useDevFlags";
import {
  updateReady,
  updateReadyVersion,
  updateInstalling,
  applyUpdateNow,
} from "../updater";
import { openDownloadsDrawer } from "../composables/useDownloadsDrawer";
import { error as devError } from "../utils/logger";
import { useI18n } from "vue-i18n";

const route = useRoute();
const router = useRouter();
const tasks = useTasksStore();
const instancesStore = useInstancesStore();
const servers = useServersStore();
const newsStore = useNewsStore();
const settings = useSettingsStore();
const message = useMessage();
const { t } = useI18n();

const isDark = computed(() => settings.settings?.theme !== "light");
function toggleTheme() {
  void settings.patch({ theme: isDark.value ? "light" : "dark" });
}

const actionIcons: Record<string, any> = {
  plus: IconPlus,
  trash: IconTrash,
};
// 页面路由 meta 中配置的导航按钮（如“新建实例”）；textKey 为文案 key
const pageAction = computed(() => (route.meta.action as { textKey?: string; icon?: string; to?: string }) ?? null);
// 标题按首个空格拆分：品牌名（QookiX）用主题色高亮，其余文案保持常规颜色
const titleParts = computed(() => {
  const full = t("titlebar.title");
  const idx = full.indexOf(" ");
  return idx > 0
    ? { brand: full.slice(0, idx), rest: full.slice(idx) }
    : { brand: full, rest: "" };
});
// 供实例页触发的“新建分组”信号（由 App 提供）
const groupDialogRequest = inject<{ value: number }>("groupDialogRequest", { value: 0 });
function requestCreateGroup() {
  groupDialogRequest.value++;
}
// 下载中数量：标题栏下载按钮角标
const downloadCount = computed(() => tasks.activeCount);

const win = getCurrentWindow();
const maximized = ref(false);

// —— 运行中的游戏：标题栏常驻一个红色「关闭」胶囊 ——
// 全局状态放标题栏，任何页面都能看到/结束，也不再挤在导航条里。
const stopLabel = computed(() =>
  tasks.runningInstances.length > 1
    ? `${t("sidebar.stopAll")} (${tasks.runningInstances.length})`
    : t("sidebar.stopAll")
);

async function stopAll() {
  try {
    await instancesStore.stop();
    message.success(t("sidebar.stopAllDone"));
  } catch (e) {
    message.error(String(e));
  }
}

/**
 * 安装已下载的更新包并重启。Windows 上 NSIS 会强制结束当前进程并自行拉起
 * 新进程，所以调用后通常不会再返回。
 */
function doApplyUpdate() {
  void applyUpdateNow().catch((err) => {
    devError("[updater] apply failed:", err);
  });
}

async function toggleMax() {
  if (await win.isMaximized()) {
    await win.unmaximize();
  } else {
    await win.maximize();
  }
  maximized.value = await win.isMaximized();
}

onMounted(async () => {
  maximized.value = await win.isMaximized();
  win.onResized(() => {
    win.isMaximized().then((m) => (maximized.value = m));
  });
});
</script>

<template>
  <div class="titlebar" id="app-titlebar" data-tauri-drag-region>
    <div class="tb-left" data-tauri-drag-region>
      <span class="tb-title"><span class="tb-brand">{{ titleParts.brand }}</span>{{ titleParts.rest }}</span>
    </div>
    <div class="tb-right" data-tauri-drag-region>
      <div class="tb-actions">
        <button
          v-if="updateReady"
          class="tb-action primary"
          :disabled="updateInstalling"
          :title="updateReadyVersion ? t('titlebar.updateTooltip', { version: updateReadyVersion }) : t('titlebar.updateTooltipGeneric')"
          @click="doApplyUpdate()"
        >
          <IconRefresh class="tb-action-icon" />
          {{ updateInstalling ? t("titlebar.updateInstalling") : t("titlebar.updateReady") }}
        </button>
        <button
          v-if="route.name === 'instances'"
          class="tb-action"
          @click="requestCreateGroup"
        >
          <IconPlus class="tb-action-icon" /> {{ t("titlebar.newGroup") }}
        </button>
        <button
          v-if="route.name === 'multiplayer' && servers.canCreate"
          class="tb-action primary"
          @click="servers.requestCreate()"
        >
          <IconPlus class="tb-action-icon" /> {{ t("titlebar.createServer") }}
        </button>
        <button
          v-if="route.name === 'browse' && newsStore.active"
          class="tb-action"
          :disabled="newsStore.loading"
          @click="newsStore.refreshSignal++"
        >
          <IconRefresh class="tb-action-icon" />
          {{ newsStore.loading ? t("news.refreshing") : t("common.refresh") }}
        </button>
        <button
          v-if="pageAction?.to"
          class="tb-action primary"
          @click="router.push(pageAction.to)"
        >
          <component :is="actionIcons[pageAction.icon ?? '']" class="tb-action-icon" />
          {{ t(pageAction.textKey ?? "") }}
        </button>
      </div>
      <!-- 游戏运行中：红色图标按钮（悬停滑出文字）。位置在「页面操作按钮」与
           「账号/窗口控制」之间——右侧这一组是右锚定的，左边界面按钮怎么增减
           都不影响它的坐标（放进 tb-actions 里会被页面按钮顶来顶去）。 -->
      <button
        v-if="tasks.gameRunning"
        class="tb-action danger tb-status-btn"
        :aria-label="t('sidebar.stopAll')"
        @click="stopAll"
      >
        <IconStop class="tb-action-icon" />
        <span class="tb-status-label">{{ stopLabel }}</span>
      </button>
      <div class="tb-controls">
        <!-- 账号：全局身份入口，与主题/下载/设置同组（圆形按钮区最左） -->
        <AccountChip bar class="tb-account" />
        <button v-if="titlebarThemeToggle" class="tb-btn tb-theme-toggle" :title="isDark ? t('titlebar.switchLight') : t('titlebar.switchDark')" @click="toggleTheme">
          <IconSun v-if="isDark" />
          <IconMoon v-else />
        </button>
        <button
          class="tb-btn tb-downloads"
          :title="t('nav.downloads')"
          @click="openDownloadsDrawer()"
        >
          <IconDownload />
          <span v-if="downloadCount > 0" class="tb-downloads-badge" :title="t('sidebar.downloading', { count: downloadCount })">{{ downloadCount }}</span>
        </button>
        <button
          class="tb-btn tb-settings"
          :title="t('nav.settings')"
          @click="router.push('/settings')"
        >
          <IconSettings />
        </button>
        <button class="tb-btn" :title="t('titlebar.minimize')" @click="win.minimize()">
        <IconMinus />
      </button>
        <button class="tb-btn" :title="maximized ? t('titlebar.restore') : t('titlebar.maximize')" @click="toggleMax">
        <IconSquare v-if="!maximized" />
        <IconRestore v-else />
      </button>
        <button class="tb-btn tb-close" :title="t('titlebar.close')" @click="win.close()">
        <IconClose />
      </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.titlebar {
  height: 40px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 8px 0 14px;
  background: var(--panel);
  backdrop-filter: blur(var(--glass-blur, 8px));
  -webkit-backdrop-filter: blur(var(--glass-blur, 8px));
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
  -webkit-app-region: drag;
}
.tb-left {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
}
/* 账号胶囊：在 tb-controls 圆形按钮组最左，高度与按钮对齐 */
.tb-account {
  flex-shrink: 0;
  -webkit-app-region: no-drag;
}
.tb-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-2);
  letter-spacing: 0.2px;
}
.tb-brand {
  font-weight: 700;
  color: var(--accent);
}
.tb-right {
  display: flex;
  align-items: center;
  gap: 10px;
}
.tb-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  -webkit-app-region: no-drag;
}
.tb-action {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 12px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--w-05);
  color: var(--text-2);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
  transition: all 0.15s;
}
.tb-action:hover:not(:disabled) {
  background: var(--w-10);
  color: var(--text-1);
}
.tb-action:disabled {
  opacity: 0.4;
  cursor: default;
}
.tb-action.primary {
  border: none;
  background: linear-gradient(135deg, var(--accent), var(--accent-deep));
  color: var(--accent-contrast);
}
.tb-action.primary:hover:not(:disabled) {
  filter: brightness(1.08);
}
/* 运行中胶囊：默认淡红底 + 红字（不抢主操作），悬停才实心红 */
.tb-action.danger {
  border: 1px solid var(--danger-40);
  background: var(--danger-12);
  color: var(--danger-text);
  white-space: nowrap;
}
.tb-action.danger:hover:not(:disabled) {
  background: var(--danger);
  border-color: var(--danger);
  color: #fff;
}
/* 运行中按钮：默认只有一个图标方块，悬停时才滑出文字（与底部 Dock 的
   nav-item 同一套做法：grid 0fr → 1fr 原生插值，图标槽固定不动）。
   位置固定在页面操作按钮与账号/窗口控制之间——右侧这组右锚定，展开时
   它向左长，右边缘与坐标都不动。 */
/* 用双类名压过 .tb-action 的 padding:0 12px / gap:6px，收起时才能是正方的 28×28 */
.tb-action.tb-status-btn {
  -webkit-app-region: no-drag;
  flex-shrink: 0;
  display: inline-grid;
  grid-template-columns: 14px 0fr;
  align-items: center;
  /* 图标与文字的间距用 column-gap：收起时是 0，不占宽度；
     若写在文字槽上用 padding-left，padding 压不掉（min-width 也压不住），
     0fr 那列会残留 6px，按钮就成 28×34 的长方形了。 */
  column-gap: 0;
  padding: 0 6px;
  overflow: hidden;
  transition:
    grid-template-columns 0.32s cubic-bezier(0.34, 1.56, 0.64, 1),
    column-gap 0.32s cubic-bezier(0.34, 1.56, 0.64, 1),
    background 0.14s,
    color 0.14s,
    border-color 0.14s;
}
.tb-action.tb-status-btn:hover {
  grid-template-columns: 14px 1fr;
  column-gap: 6px;
}
.tb-action.tb-status-btn .tb-action-icon {
  grid-column: 1;
  justify-self: center;
}
/* 文字槽：0fr 时宽度为 0、被 overflow 裁掉；间距由上面的 column-gap 提供 */
.tb-status-label {
  grid-column: 2;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  opacity: 0;
  transform: translateX(-8px);
  transition:
    transform 0.26s cubic-bezier(0.55, 0, 0.55, 0.2),
    opacity 0.18s ease;
}
.tb-status-btn:hover .tb-status-label {
  opacity: 1;
  transform: translateX(0);
  transition:
    transform 0.32s cubic-bezier(0.34, 1.56, 0.64, 1) 0.02s,
    opacity 0.18s ease 0.04s;
}

.tb-action-icon {
  width: 14px;
  height: 14px;
}
.tb-controls {
  display: flex;
  gap: 2px;
  -webkit-app-region: no-drag;
}
.tb-btn {
  width: 30px;
  height: 30px;
  border: none;
  background: transparent;
  color: var(--text-3);
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 15px;
  cursor: pointer;
  transition: all 0.12s;
}
.tb-btn:hover {
  background: var(--panel-hover);
  color: var(--text-1);
}
/* 下载按钮：主题色角标显示进行中的下载数 */
.tb-downloads {
  position: relative;
}
.tb-downloads-badge {
  position: absolute;
  top: -2px;
  right: -2px;
  min-width: 15px;
  height: 15px;
  padding: 0 4px;
  border-radius: 8px;
  background: var(--accent);
  color: var(--accent-contrast);
  font-size: 10px;
  font-weight: 700;
  line-height: 15px;
  text-align: center;
  box-sizing: border-box;
  pointer-events: none;
}
.tb-close:hover {
  background: #e5534b;
  color: #fff;
}
.restore-icon {
  transform: scale(0.75);
  display: block;
}
</style>
