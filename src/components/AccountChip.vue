<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { NPopover, useMessage } from "naive-ui";
import { useAccountsStore } from "../stores/accounts";
import { useAccountAvatars } from "../composables/useAccountAvatars";
import { IconCheck, IconChevronDown, IconUser, IconSettings } from "./icons";
import type { Account } from "../types";

const props = defineProps<{ bar?: boolean }>();

const accounts = useAccountsStore();
const message = useMessage();
const { t } = useI18n();
const { avatar, onAvatarError } = useAccountAvatars();

const popoverShow = ref(false);
const current = computed(() => accounts.current);

/** 点击芯片：打开账号管理抽屉（完整增删），并收起悬停 Popover。 */
function openDrawer() {
  popoverShow.value = false;
  accounts.showManager = true;
}

async function select(acc: Account) {
  await accounts.select(acc.uuid);
  message.success(t("accountChip.currentPlaying", { username: acc.username }));
  popoverShow.value = false;
}

function typeLabel(a: Account) {
  if (a.type === "microsoft") return t("accountChip.typeMicrosoft");
  if (a.type === "yggdrasil") return a.server_name || t("accountChip.typeYggdrasil");
  return t("accountChip.typeOffline");
}
</script>

<template>
  <n-popover
    v-model:show="popoverShow"
    trigger="hover"
    :placement="props.bar ? 'bottom-start' : 'top-start'"
    :width="300"
    :show-arrow="false"
    class="acctp-popover"
  >
    <template #trigger>
      <div
        id="account-chip"
        class="acct-chip clickable"
        :class="{ empty: !accounts.accounts.length, bar: props.bar }"
        @click="openDrawer"
      >
        <div class="avatar">
          <template v-if="current">
            <img v-if="avatar(current.uuid)" :src="avatar(current.uuid)" alt="" @error="onAvatarError(current.uuid)" />
            <IconUser v-else />
          </template>
          <IconUser v-else />
        </div>
        <div class="acct-info">
          <div class="acct-name text-ellipsis">{{ current?.username ?? t("accountChip.notLoggedIn") }}</div>
          <div class="acct-type">
            {{
              current
                ? current.type === "microsoft"
                  ? t("accountChip.microsoftAccount")
                  : current.type === "yggdrasil"
                    ? current.server_name
                    : t("accountChip.offlineAccount")
                : t("accountChip.clickToAddAccount")
            }}
          </div>
        </div>
        <IconChevronDown v-if="accounts.accounts.length" class="chev" />
        <IconUser v-else class="chev" />
      </div>
    </template>

    <div class="acctp-body">
      <div class="acctp-title">{{ t("accountChip.currentPlayingLabel") }}</div>

      <div v-if="!accounts.accounts.length" class="acctp-empty">{{ t("accountChip.noAccount") }}</div>
      <div v-else class="acctp-list">
        <div
          v-for="acc in accounts.accounts"
          :key="acc.uuid"
          class="acctp-row"
          :class="{ active: current?.uuid === acc.uuid }"
          @click="select(acc)"
        >
          <img
            v-if="avatar(acc.uuid)"
            :src="avatar(acc.uuid)"
            class="acctp-avatar"
            alt=""
            @error="onAvatarError(acc.uuid)"
          />
          <IconUser v-else class="acctp-avatar acctp-avatar-fallback" />
          <div class="acctp-info">
            <div class="acctp-name text-ellipsis">{{ acc.username }}</div>
            <span class="acctp-type" :class="acc.type">{{ typeLabel(acc) }}</span>
          </div>
          <IconCheck v-if="current?.uuid === acc.uuid" class="acctp-check" />
        </div>
      </div>

      <div class="acctp-divider"></div>

      <button class="acctp-manage-btn" @click="openDrawer">
        <IconSettings /> {{ t("accountChip.manageAccounts") }}
      </button>
    </div>
  </n-popover>
</template>

<style scoped>
/* chip（原位渲染，scoped 生效） */
.acct-chip {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 10px;
  height: 48px;
  box-sizing: border-box;
  border-radius: 12px;
  background: var(--panel);
  border: 1px solid var(--border);
}
.acct-chip.empty {
  opacity: 0.85;
}
/* 标题栏变体：小头像按钮，无边框无背景，与标题栏圆形按钮组同高对齐 */
.acct-chip.bar {
  height: 30px;
  padding: 0 8px;
  gap: 7px;
  background: transparent;
  border: none;
  border-radius: 999px;
}
.acct-chip.bar .avatar {
  width: 22px;
  height: 22px;
  border-radius: 50%;
}
.acct-chip.bar .acct-name {
  font-size: 12px;
}
.acct-chip.bar .acct-type {
  display: none;
}
.acct-chip.bar:hover {
  background: var(--panel-hover);
}
.avatar {
  width: 34px;
  height: 34px;
  border-radius: 8px;
  overflow: hidden;
  background: var(--w-08);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
  font-size: 17px;
  flex-shrink: 0;
}
.avatar img {
  width: 100%;
  height: 100%;
  image-rendering: pixelated;
}
.acct-info {
  min-width: 0;
  flex: 1;
  animation: fade-in 0.2s ease;
}
.acct-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-1);
}
.acct-type {
  font-size: 11px;
  color: var(--text-3);
}
.chev {
  color: var(--text-3);
  font-size: 14px;
  flex-shrink: 0;
  animation: fade-in 0.2s ease;
}
@keyframes fade-in {
  from { opacity: 0; }
  to { opacity: 1; }
}
</style>

<!-- Popover 内容被 naive-ui teleport 到 <body>，scoped 不可靠，故用全局类名 acctp-* -->
<style>
.acctp-popover {
  padding: 12px;
}
.acctp-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.acctp-title {
  font-size: 12px;
  font-weight: 700;
  color: var(--text-3);
  letter-spacing: 0.5px;
}
.acctp-empty {
  font-size: 12px;
  color: var(--text-3);
  padding: 6px 0;
}
.acctp-list {
  display: flex;
  flex-direction: column;
  gap: 3px;
  max-height: 240px;
  overflow-y: auto;
}
.acctp-row {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 6px 8px;
  border-radius: 9px;
  cursor: pointer;
  border: 1px solid transparent;
  transition: all 0.12s;
}
.acctp-row:hover {
  background: var(--w-06);
}
.acctp-row.active {
  background: var(--accent-14);
  border-color: var(--accent-35);
}
.acctp-avatar {
  width: 28px;
  height: 28px;
  border-radius: 8px;
  image-rendering: pixelated;
  background: var(--w-06);
  flex-shrink: 0;
}
.acctp-avatar-fallback {
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
  font-size: 14px;
}
.acctp-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.acctp-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-1);
}
.acctp-type {
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 6px;
  font-weight: 600;
  align-self: flex-start;
}
.acctp-type.microsoft {
  background: var(--info-15);
  color: var(--text-2);
}
.acctp-type.yggdrasil {
  background: var(--success-12);
  color: var(--success);
}
.acctp-type.offline {
  background: var(--w-08);
  color: var(--text-2);
}
.acctp-check {
  color: var(--accent);
  font-size: 14px;
  flex-shrink: 0;
}
.acctp-divider {
  border-top: 1px solid var(--w-08);
  margin: 2px 0;
}
.acctp-manage-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  width: 100%;
  border: 1px solid var(--w-08);
  background: var(--w-05);
  color: var(--text-1);
  border-radius: 9px;
  padding: 8px 12px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
  white-space: nowrap;
  transition: all 0.12s;
}
.acctp-manage-btn:hover {
  background: var(--w-10);
}
</style>
