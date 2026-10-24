<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { NButton, NInput, NModal, NSelect, NTabs, NTabPane, useMessage } from "naive-ui";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useAccountsStore } from "../stores/accounts";
import { useI18n } from "vue-i18n";
import { api } from "../api";
import type { Account } from "../types";

const emit = defineEmits<{ added: [acc?: Account] }>();
const accounts = useAccountsStore();
const message = useMessage();
const { t } = useI18n();

const show = ref(false);
const activeTab = ref<"offline" | "ygg" | "microsoft">("microsoft");

// ---- 离线账号 ----
const offlineName = ref("");
const addingOffline = ref(false);
const showAdvanced = ref(false);
const offlineUuid = ref("");

async function addOffline() {
  const name = offlineName.value.trim();
  if (!name) {
    message.warning(t("accountChip.pleaseEnterUsername"));
    return;
  }
  const uuid = offlineUuid.value.trim();
  addingOffline.value = true;
  try {
    await accounts.addOffline(name, uuid || undefined);
    message.success(t("accountChip.offlineAccountAdded"));
    offlineName.value = "";
    offlineUuid.value = "";
    showAdvanced.value = false;
    show.value = false;
    emit("added");
  } catch (e) {
    message.error(String(e));
  } finally {
    addingOffline.value = false;
  }
}

// ---- 第三方认证（authlib-injector / Yggdrasil） ----
const LITTLE_ROOT = "https://littleskin.cn/api/yggdrasil";
const LITTLE_LINKS = { homepage: "https://littleskin.cn", register: "https://littleskin.cn/auth/register" };
const SHIMU_ROOT = "https://mcimg.com/shimu/yggdrasil";
const SHIMU_LINKS = { homepage: "https://www.mcimg.com", register: "" };

const serverChoice = ref<"little" | "shimu" | "custom">("little");
const customRoot = ref("");
const yggUsername = ref("");
const yggPassword = ref("");
const yggLoggingIn = ref(false);
const yggAdding = ref(false);

interface LoginResult {
  accessToken: string;
  clientToken: string;
  server: string;
  serverName: string;
  links: { homepage: string; register: string };
  profiles: { id: string; name: string }[];
}
const yggResult = ref<LoginResult | null>(null);
const chosenProfile = ref("");

const serverOptions = computed(() => [
  { label: "LittleSkin", value: "little" },
  { label: "拾木空间", value: "shimu" },
  { label: t("yggLogin.customServer"), value: "custom" },
]);
const PRESET_ROOTS: Record<"little" | "shimu", string> = { little: LITTLE_ROOT, shimu: SHIMU_ROOT };
const PRESET_LABELS: Record<"little" | "shimu", string> = { little: "LittleSkin", shimu: "拾木空间" };
const PRESET_LINKS: Record<"little" | "shimu", { homepage: string; register: string }> = {
  little: LITTLE_LINKS,
  shimu: SHIMU_LINKS,
};
const apiRoot = computed(() =>
  serverChoice.value === "custom" ? customRoot.value.trim() : PRESET_ROOTS[serverChoice.value as "little" | "shimu"]
);
const serverLabel = computed(() => {
  if (serverChoice.value === "custom") {
    return yggResult.value?.serverName || customRoot.value.trim() || t("yggLogin.customServer");
  }
  return PRESET_LABELS[serverChoice.value as "little" | "shimu"];
});
const homepageUrl = computed(() => {
  if (serverChoice.value === "custom") return yggResult.value?.links.homepage || "";
  return PRESET_LINKS[serverChoice.value as "little" | "shimu"].homepage;
});
const registerUrl = computed(() => {
  if (serverChoice.value === "custom") return yggResult.value?.links.register || "";
  return PRESET_LINKS[serverChoice.value as "little" | "shimu"].register;
});

function openLink(url: string) {
  if (url) openUrl(url).catch((e: unknown) => message.error(t("yggLogin.openFailed", { error: String(e) })));
}

async function yggLogin() {
  if (!apiRoot.value) {
    message.warning(t("yggLogin.fillServerUrl"));
    return;
  }
  yggLoggingIn.value = true;
  try {
    yggResult.value = await api.yggdrasilLogin(apiRoot.value, yggUsername.value, yggPassword.value);
    const profiles = yggResult.value.profiles;
    chosenProfile.value = profiles.length === 1 ? profiles[0].id : "";
    if (profiles.length === 1) await yggAdd();
  } catch (e) {
    message.error(String(e));
  } finally {
    yggLoggingIn.value = false;
  }
}

async function yggAdd() {
  const r = yggResult.value;
  if (!r) return;
  const profile = r.profiles.find((p) => p.id === chosenProfile.value);
  if (!profile) {
    message.warning(t("yggLogin.selectProfile"));
    return;
  }
  yggAdding.value = true;
  try {
    const acc = await api.yggdrasilAddAccount({
      serverUrl: r.server,
      serverName: r.serverName,
      accessToken: r.accessToken,
      clientToken: r.clientToken,
      profileId: profile.id,
      profileName: profile.name,
    });
    await accounts.refresh();
    await accounts.select(acc.uuid);
    message.success(t("accountChip.yggAccountAdded", { username: acc.username }));
    show.value = false;
    emit("added", acc);
  } catch (e) {
    message.error(String(e));
  } finally {
    yggAdding.value = false;
  }
}

// ---- 微软账号（设备码 OAuth） ----
const msStarting = ref(false);

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const ta = document.createElement("textarea");
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    document.body.removeChild(ta);
  }
}

async function startMs() {
  if (msStarting.value || accounts.msPolling) return;
  msStarting.value = true;
  accounts.msError = "";
  try {
    await accounts.startMs();
    if (accounts.msError) {
      message.error(accounts.msError);
      accounts.msError = "";
    }
  } catch (e) {
    message.error(String(e));
  } finally {
    msStarting.value = false;
  }
}

async function copyMsCode() {
  if (accounts.msFlow?.userCode) await copyText(accounts.msFlow.userCode);
}

async function retryMs() {
  accounts.msError = "";
  await startMs();
}

function buyMinecraft() {
  openUrl("https://www.xbox.com/zh-CN/games/store/productId/9NXP44L49SHJ").catch(() => {});
}

// 设备码出现时自动复制并打开浏览器（仅在微软 tab 激活时）
watch(
  () => accounts.msFlow,
  (f) => {
    if (f && activeTab.value === "microsoft" && show.value) {
      if (f.userCode) copyText(f.userCode).catch(() => {});
      if (f.verificationUri) openUrl(f.verificationUri).catch(() => {});
    }
  }
);

function open() {
  activeTab.value = "microsoft";
  offlineName.value = "";
  serverChoice.value = "little";
  customRoot.value = "";
  yggUsername.value = "";
  yggPassword.value = "";
  yggResult.value = null;
  chosenProfile.value = "";
  accounts.msFlow = null;
  accounts.msError = "";
  show.value = true;
}

function close() {
  show.value = false;
  accounts.msFlow = null;
  accounts.msError = "";
}

defineExpose({ open, close });
</script>

<template>
  <n-modal
    v-model:show="show"
    preset="card"
    :title="t('accountChip.addAccountTitle')"
    style="width: 480px; max-width: 92vw"
  >
    <n-tabs v-model:value="activeTab" type="segment" size="medium" animated>
      <!-- 微软账号 -->
      <n-tab-pane name="microsoft" :tab="t('accountChip.addMicrosoftAccount')">
        <div class="qk-add-box qkms-box">
          <template v-if="accounts.msError">
            <div class="qkms-error-box">{{ accounts.msError }}</div>
            <n-button type="primary" block @click="retryMs">{{ t("msLogin.retry") }}</n-button>
          </template>
          <template v-else-if="accounts.msFlow">
            <p>{{ t("msLogin.autoCopiedHint") }}</p>
            <a class="qkms-link" @click="accounts.msFlow && openUrl(accounts.msFlow.verificationUri)">
              {{ accounts.msFlow.verificationUri }}
            </a>
            <div class="qkms-code mono">{{ accounts.msFlow.userCode || t("msLogin.waitingCode") }}</div>
            <p class="qkms-hint">{{ t("msLogin.waitingAuth") }}</p>
            <div v-if="accounts.msPolling" class="qkms-polling">{{ t("msLogin.polling") }}</div>
            <div class="qkms-footer">
              <n-button @click="copyMsCode">{{ t("msLogin.copyCode") }}</n-button>
              <n-button type="primary" @click="accounts.manualCheck">{{ t("msLogin.iLoggedIn") }}</n-button>
            </div>
          </template>
          <template v-else>
            <p class="qk-add-hint">{{ t("msLogin.startHint") }}</p>
            <n-button type="primary" block :loading="msStarting" @click="startMs">
              {{ t("msLogin.startLogin") }}
            </n-button>
          </template>
        </div>
      </n-tab-pane>

      <!-- 离线账号 -->
      <n-tab-pane name="offline" :tab="t('accountChip.addOfflineAccount')">
        <div class="qk-add-box">
          <label class="qk-field-label">{{ t("accountChip.usernameLabel") }}</label>
          <n-input
            v-model:value="offlineName"
            :placeholder="t('accountChip.offlineNamePlaceholder')"
            :maxlength="16"
            clearable
            @keyup.enter="addOffline"
          />
          <p class="qk-add-hint">{{ t("accountChip.offlineHint") }}</p>
          <div class="qk-advanced">
            <button type="button" class="qk-advanced-toggle" @click="showAdvanced = !showAdvanced">
              <span class="qk-chevron" :class="{ open: showAdvanced }">▸</span>
              {{ t("accountChip.advanced") }}
            </button>
            <div v-if="showAdvanced" class="qk-advanced-body">
              <label class="qk-field-label">{{ t("accountChip.customUuid") }}</label>
              <n-input
                v-model:value="offlineUuid"
                :placeholder="t('accountChip.uuidPlaceholder')"
                clearable
              />
            </div>
          </div>
          <n-button type="primary" block :loading="addingOffline" @click="addOffline">
            {{ t("accountChip.add") }}
          </n-button>
        </div>
      </n-tab-pane>

      <!-- 第三方认证 -->
      <n-tab-pane name="ygg" :tab="t('accountChip.addYggAccount')">
        <div class="qk-add-box">
          <template v-if="!yggResult">
            <label class="qk-field-label">{{ t("accountChip.authSourceLabel") }}</label>
            <n-select
              v-model:value="serverChoice"
              :options="serverOptions"
              :placeholder="t('yggLogin.selectServer')"
            />
            <n-input
              v-if="serverChoice === 'custom'"
              v-model:value="customRoot"
              :placeholder="t('yggLogin.apiPlaceholder')"
            />
            <div class="ygg-server">
              <span class="ygg-server-label">{{ t("yggLogin.authServer") }}</span>
              <span class="ygg-server-name">{{ serverLabel }}</span>
              <span class="ygg-server-links">
                <a v-if="homepageUrl" class="ygg-link" @click.prevent="openLink(homepageUrl)">{{ t("yggLogin.homepage") }}</a>
                <a v-if="registerUrl" class="ygg-link" @click.prevent="openLink(registerUrl)">{{ t("yggLogin.register") }}</a>
              </span>
            </div>
            <n-input
              v-model:value="yggUsername"
              :placeholder="t('yggLogin.usernamePlaceholder')"
              @keyup.enter="yggLogin"
            />
            <n-input
              v-model:value="yggPassword"
              type="password"
              show-password-on="click"
              :placeholder="t('yggLogin.passwordPlaceholder')"
              @keyup.enter="yggLogin"
            />
            <p class="qk-add-hint">{{ t("yggLogin.hint") }}</p>
            <n-button type="primary" block :loading="yggLoggingIn" @click="yggLogin">
              {{ yggLoggingIn ? t("yggLogin.loggingIn") : t("yggLogin.login") }}
            </n-button>
          </template>
          <template v-else>
            <p class="qk-add-hint">{{ t("yggLogin.loginSuccess", { server: yggResult.serverName }) }}</p>
            <div class="ygg-profiles">
              <button
                v-for="p in yggResult.profiles"
                :key="p.id"
                class="ygg-profile"
                :class="{ on: chosenProfile === p.id }"
                @click="chosenProfile = p.id"
              >
                <span class="ygg-pname">{{ p.name }}</span>
                <span class="ygg-pid">{{ p.id }}</span>
              </button>
            </div>
            <n-button type="primary" block :loading="yggAdding" @click="yggAdd">
              {{ t("yggLogin.addProfile") }}
            </n-button>
          </template>
        </div>
      </n-tab-pane>
    </n-tabs>
    <div class="qk-buy-row">
      <a class="qk-buy-link" @click.prevent="buyMinecraft">{{ t("accountChip.buyMinecraft") }}</a>
    </div>
  </n-modal>
</template>

<style>
.qk-add-box {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding-top: 4px;
}
.qk-add-hint {
  margin: 0;
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-3);
}
.qk-field-label {
  display: block;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-2);
}
.qk-buy-row {
  display: flex;
  justify-content: center;
  margin-top: 12px;
  padding-top: 10px;
  border-top: 1px solid var(--w-08);
}

/* 离线账号高级选项 */
.qk-advanced {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.qk-advanced-toggle {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: none;
  background: transparent;
  color: var(--text-3);
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  padding: 0;
  font-family: inherit;
  transition: color 0.12s;
}
.qk-advanced-toggle:hover {
  color: var(--text-1);
}
.qk-chevron {
  display: inline-block;
  transition: transform 0.18s ease;
  font-size: 11px;
}
.qk-chevron.open {
  transform: rotate(90deg);
}
.qk-advanced-body {
  display: flex;
  flex-direction: column;
  gap: 10px;
  animation: qk-adv-in 0.18s ease;
}
@keyframes qk-adv-in {
  from { opacity: 0; transform: translateY(-4px); }
  to { opacity: 1; transform: translateY(0); }
}

/* 购买 Minecraft 链接 */
.qk-buy-link {
  text-align: center;
  font-size: 12px;
  color: var(--text-3);
  cursor: pointer;
  text-decoration: underline;
  transition: color 0.12s;
}
.qk-buy-link:hover {
  color: var(--accent);
}

/* 第三方认证 */
.ygg-server {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
}
.ygg-server-label {
  color: var(--text-3);
}
.ygg-server-name {
  color: var(--text-1);
}
.ygg-server-links {
  margin-left: auto;
  display: inline-flex;
  gap: 12px;
}
.ygg-link {
  color: var(--accent);
  cursor: pointer;
  text-decoration: none;
}
.ygg-link:hover {
  text-decoration: underline;
}
.ygg-profiles {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.ygg-profile {
  display: flex;
  flex-direction: column;
  gap: 2px;
  text-align: left;
  padding: 9px 12px;
  border-radius: 10px;
  border: 1px solid var(--w-08);
  background: var(--w-04);
  cursor: pointer;
  font-family: inherit;
}
.ygg-profile:hover {
  border-color: var(--w-14);
}
.ygg-profile.on {
  border-color: var(--accent);
  background: var(--accent-08);
}
.ygg-pname {
  color: var(--text-1);
  font-size: 13px;
}
.ygg-pid {
  color: var(--text-3);
  font-size: 11px;
  font-family: "Cascadia Code", Consolas, monospace;
}

/* 微软设备码 */
.qkms-box p {
  margin: 0;
  font-size: 13px;
  color: #c6c8d2;
  text-align: center;
}
.qkms-link {
  color: #e89a4b;
  font-size: 14px;
  word-break: break-all;
  cursor: pointer;
  text-decoration: underline;
}
.qkms-code {
  font-size: 30px;
  font-weight: 800;
  letter-spacing: 10px;
  text-align: center;
  color: #e89a4b;
  background: var(--accent-08);
  border: 1px dashed var(--accent-04);
  border-radius: 12px;
  padding: 14px 0;
  width: 100%;
}
.qkms-hint {
  font-size: 12px;
  color: #8b8e9c;
}
.qkms-polling {
  font-size: 12px;
  color: #e89a4b;
}
.qkms-error-box {
  color: var(--danger);
  font-size: 13px;
  line-height: 1.6;
  word-break: break-all;
  user-select: text;
  -webkit-user-select: text;
  cursor: text;
  background: var(--danger-08);
  border: 1px solid var(--danger-30);
  border-radius: 8px;
  padding: 12px;
  width: 100%;
}
.qkms-footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}
</style>
