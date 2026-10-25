<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useMessage } from "naive-ui";
import { useAccountsStore } from "../stores/accounts";
import { useAccountAvatars } from "../composables/useAccountAvatars";
import AddAccountDialog from "./AddAccountDialog.vue";
import { IconUsers, IconClose, IconTrash, IconCheck, IconPlus, IconUser } from "./icons";
import type { Account } from "../types";

const accounts = useAccountsStore();
const message = useMessage();
const { t } = useI18n();
const { avatar, onAvatarError } = useAccountAvatars();

const current = computed(() => accounts.current);

function close() {
  accounts.showManager = false;
}

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") close();
}
onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));

async function select(acc: Account) {
  await accounts.select(acc.uuid);
  message.success(t("accountChip.currentPlaying", { username: acc.username }));
}

function remove(acc: Account) {
  accounts
    .remove(acc.uuid)
    .then(() => message.success(t("accountChip.accountRemoved")))
    .catch((e) => message.error(String(e)));
}

function typeLabel(a: Account) {
  if (a.type === "microsoft") return t("accountChip.typeMicrosoft");
  if (a.type === "yggdrasil") return a.server_name || t("accountChip.typeYggdrasil");
  return t("accountChip.typeOffline");
}

// ---- 添加账号 ----
const addDialog = ref<InstanceType<typeof AddAccountDialog> | null>(null);
function openAdd() {
  addDialog.value?.open();
}

// MS 登录结果反馈（device-code 轮询在 store 里跑，这里只展示 + 收尾）
watch(
  () => accounts.msSuccess,
  (msg) => {
    if (msg) {
      message.success(msg);
      accounts.msSuccess = "";
      addDialog.value?.close();
    }
  },
);
watch(
  () => accounts.msFailed,
  (msg) => {
    if (msg) {
      message.error(msg, { duration: 8000 });
      accounts.msFailed = "";
    }
  },
);
</script>

<template>
  <Teleport to="body">
    <!-- 半透明遮罩：点击关闭 -->
    <Transition name="acct-drawer-fade">
      <div v-if="accounts.showManager" class="acct-mask" @click="close"></div>
    </Transition>

    <!-- 右抽屉本体：悬浮岛样式（右侧留空 + 圆角 + 环绕投影 + 磨砂）。
         ⚠ 滑入用 right（layout）而不是 transform：transform 会创建独立
         合成层，抽屉的 backdrop-filter 磨砂在滑动期间采样不到背后内容。 -->
    <Transition name="acct-drawer-slide">
      <aside v-if="accounts.showManager" class="acct-drawer glass">
        <header class="acct-head">
          <div class="acct-title">
            <IconUsers class="acct-title-icon" />
            <span>{{ t("accountChip.drawerTitle") }}</span>
          </div>
          <button class="acct-close-btn" :title="t('titlebar.close')" @click="close">
            <IconClose />
          </button>
        </header>

        <div class="acct-body">
          <div v-if="!accounts.accounts.length" class="acct-empty">{{ t("accountChip.noAccount") }}</div>
          <div v-else class="acct-list">
            <div
              v-for="acc in accounts.accounts"
              :key="acc.uuid"
              class="acct-row"
              :class="{ active: current?.uuid === acc.uuid }"
              @click="select(acc)"
            >
              <img
                v-if="avatar(acc.uuid)"
                :src="avatar(acc.uuid)"
                class="acct-avatar"
                alt=""
                @error="onAvatarError(acc.uuid)"
              />
              <IconUser v-else class="acct-avatar acct-avatar-fallback" />
              <div class="acct-info">
                <div class="acct-name text-ellipsis">{{ acc.username }}</div>
                <span class="acct-type" :class="acc.type">{{ typeLabel(acc) }}</span>
              </div>
              <IconCheck v-if="current?.uuid === acc.uuid" class="acct-check" />
              <button class="acct-remove" :title="t('accountChip.removeAccount')" @click.stop="remove(acc)">
                <IconTrash />
              </button>
            </div>
          </div>
        </div>

        <div class="acct-foot">
          <button class="acct-add-btn" @click="openAdd">
            <IconPlus /> {{ t("accountChip.addAccountTitle") }}
          </button>
        </div>
      </aside>
    </Transition>

    <AddAccountDialog ref="addDialog" />
  </Teleport>
</template>

<style scoped>
/* —— 遮罩 —— */
.acct-mask {
  position: fixed;
  inset: 0;
  z-index: 90;
  background: rgba(0, 0, 0, 0.18);
}
/* —— 抽屉本体 —— */
.acct-drawer {
  position: fixed;
  top: 50px; /* 标题栏 40px + 10px 悬浮间距 */
  bottom: 10px;
  right: 10px;
  width: 380px;
  max-width: calc(100vw - 40px);
  z-index: 95;
  display: flex;
  flex-direction: column;
  padding: 14px 14px 12px;
  border: 1px solid var(--border);
  border-radius: 18px;
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.18);
  overflow: hidden;
}
.acct-head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
  flex-shrink: 0;
}
.acct-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 15px;
  font-weight: 700;
  color: var(--text-1);
  flex: 1;
  min-width: 0;
}
.acct-title-icon {
  font-size: 16px;
  color: var(--accent);
}
.acct-close-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  padding: 0;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: var(--text-3);
  font-size: 14px;
  cursor: pointer;
  transition: all 0.12s;
}
.acct-close-btn:hover {
  background: var(--panel-hover);
  color: var(--text-1);
}
.acct-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 10px;
  overflow-y: auto;
}
.acct-empty {
  font-size: 13px;
  color: var(--text-3);
  padding: 12px 0;
  text-align: center;
}
.acct-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.acct-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 9px;
  border-radius: 10px;
  cursor: pointer;
  border: 1px solid transparent;
  transition: all 0.12s;
}
.acct-row:hover {
  background: var(--w-06);
}
.acct-row.active {
  background: var(--accent-14);
  border-color: var(--accent-35);
}
.acct-avatar {
  width: 34px;
  height: 34px;
  border-radius: 9px;
  image-rendering: pixelated;
  background: var(--w-06);
  flex-shrink: 0;
}
.acct-avatar-fallback {
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
  font-size: 16px;
}
.acct-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.acct-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-1);
}
.acct-type {
  font-size: 10px;
  padding: 1px 7px;
  border-radius: 6px;
  font-weight: 600;
  align-self: flex-start;
}
.acct-type.microsoft {
  background: var(--info-15);
  color: var(--text-2);
}
.acct-type.yggdrasil {
  background: var(--success-12);
  color: var(--success);
}
.acct-type.offline {
  background: var(--w-08);
  color: var(--text-2);
}
.acct-check {
  color: var(--accent);
  font-size: 15px;
  flex-shrink: 0;
}
.acct-remove {
  width: 28px;
  height: 28px;
  border: none;
  background: transparent;
  color: var(--text-3);
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  font-size: 13px;
  opacity: 0;
  transition: all 0.12s;
}
.acct-row:hover .acct-remove {
  opacity: 1;
}
.acct-remove:hover {
  color: var(--danger);
  background: var(--danger-12);
}
.acct-foot {
  flex-shrink: 0;
  margin-top: 4px;
  padding-top: 10px;
  border-top: 1px solid var(--w-08);
}
.acct-add-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  width: 100%;
  border: 1px solid var(--accent-45);
  background: var(--accent-14);
  color: var(--accent);
  border-radius: 10px;
  padding: 10px 12px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
  white-space: nowrap;
  transition: all 0.12s;
}
.acct-add-btn:hover {
  background: var(--accent-22);
}

/* —— 遮罩淡入淡出 —— */
.acct-drawer-fade-enter-active,
.acct-drawer-fade-leave-active {
  transition: opacity 0.2s ease;
}
.acct-drawer-fade-enter-from,
.acct-drawer-fade-leave-to {
  opacity: 0;
}
/* —— 抽屉滑入滑出（right 位移，layout 属性，磨砂全程有效） —— */
.acct-drawer-slide-enter-active {
  transition: right 0.26s cubic-bezier(0.22, 1, 0.36, 1);
}
.acct-drawer-slide-leave-active {
  transition: right 0.2s ease-in;
}
.acct-drawer-slide-enter-from,
.acct-drawer-slide-leave-to {
  right: -400px;
}
</style>
