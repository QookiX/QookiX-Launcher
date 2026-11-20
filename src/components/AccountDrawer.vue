<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useMessage } from "naive-ui";
import { useAccountsStore } from "../stores/accounts";
import { useAccountAvatars } from "../composables/useAccountAvatars";
import FloatingDrawer from "./FloatingDrawer.vue";
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
  <FloatingDrawer :open="accounts.showManager" :label="t('accountChip.drawerTitle')" @close="close">
        <header class="acct-head">
          <div class="acct-title">
            <IconUsers class="acct-title-icon" />
            <span>{{ t("accountChip.drawerTitle") }}</span>
          </div>
          <button class="acct-close-btn" :aria-label="t('titlebar.close')" :title="t('titlebar.close')" data-drawer-close @click="close">
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
            >
              <button type="button" class="acct-select" :aria-pressed="current?.uuid === acc.uuid" @click="select(acc)">
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
              </button>
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
  </FloatingDrawer>
  <AddAccountDialog ref="addDialog" />
</template>

<style scoped>
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
  font-size: var(--fs-md);
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
  font-size: var(--fs-base);
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
  font-size: var(--fs-sm);
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
  border-radius: var(--r-md);
  cursor: pointer;
  border: 1px solid transparent;
  transition: all 0.12s;
}
.acct-select {
  display: flex;
  align-items: center;
  gap: 10px;
  flex: 1;
  min-width: 0;
  text-align: left;
  background: transparent;
  border: 0;
  padding: 0;
  cursor: pointer;
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
  border-radius: var(--r-sm);
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
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
}
.acct-type {
  font-size: var(--fs-2xs);
  padding: 1px 7px;
  border-radius: var(--r-xs);
  font-weight: 600;
  align-self: flex-start;
}
.acct-type.microsoft {
  background: var(--info-22);
  color: var(--info);
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
  font-size: var(--fs-md);
  flex-shrink: 0;
}
.acct-remove {
  width: 28px;
  height: 28px;
  border: none;
  background: transparent;
  color: var(--text-3);
  border-radius: var(--r-sm);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  font-size: var(--fs-sm);
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
  border-radius: var(--r-md);
  padding: 10px 12px;
  font-size: var(--fs-sm);
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
</style>
