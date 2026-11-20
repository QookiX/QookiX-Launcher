<script setup lang="ts">
import { ref, watch } from "vue";
import { NInput, useMessage } from "naive-ui";
import { useSettingsStore } from "../../stores/settings";
import { useAccountsStore } from "../../stores/accounts";
import { useI18n } from "vue-i18n";
import { allowDevTools, setAllowDevTools, titlebarThemeToggle, devMode, frpEnabled, setFrpEnabled } from "../../composables/useDevFlags";

import SettingsUpdateDialog from "./SettingsUpdateDialog.vue";

const settings = useSettingsStore();
const { t } = useI18n();
const message = useMessage();
const accounts = useAccountsStore();
const emit = defineEmits<{ setDevMode: [boolean]; reloadChangelog: [] }>();
const previewUpdate = ref(false);

/**
 * 微软登录用的 OAuth Client ID：正式包由 CI 的 `MS_CLIENT_ID` 编译期注入
 * （`option_env!`），开发构建没带就只能在「开发者工具」页临时填一个，写进本地设置。
 */
const showMsClientId = import.meta.env.DEV;
const msClientIdDraft = ref("");
const msClientIdSaving = ref(false);
watch(
  () => settings.settings?.ms_client_id,
  (v) => {
    if (v && !msClientIdDraft.value) msClientIdDraft.value = v;
  },
  { immediate: true },
);

async function saveMsClientId() {
  const v = msClientIdDraft.value.trim();
  if (!v) {
    message.warning(t("settings.dev.msCidEmpty"));
    return;
  }
  msClientIdSaving.value = true;
  try {
    await settings.patch({ ms_client_id: v });
    message.success(t("settings.dev.msCidSaved"));
  } catch (e) {
    message.error(String(e));
  } finally {
    msClientIdSaving.value = false;
  }
}

// ---- 开发者工具：OAuth2 接入凭证 ----
const devLsClientId = ref("");
const devLsClientSecret = ref("");
const devCredSaving = ref(false);

/** 从已加载的 accounts 配置回填到输入框 */
function syncDevCredentials() {
  devLsClientId.value = accounts.yggOauthConfig?.clientId ?? "";
  // secret 不回传，只清空输入框；用 secretConfigured 标识展示是否已设
  devLsClientSecret.value = "";
}

async function loadDevCredentials() {
  await accounts.loadYggOauthConfig();
  syncDevCredentials();
}

async function saveDevLsCredentials() {
  devCredSaving.value = true;
  try {
    const id = devLsClientId.value.trim();
    const sec = devLsClientSecret.value.trim();
    // secret 留空表示保持原值不变
    await accounts.setYggOauthCredentials(id, sec || undefined);
    syncDevCredentials();
    message.success(t("settings.dev.credSaved"));
  } catch (e) {
    message.error(String(e));
  } finally {
    devCredSaving.value = false;
  }
}

// 进入开发者模式时拉一次凭证回填；immediate 覆盖「上次已持久化开启」的情况
watch(devMode, (on) => { if (on) void loadDevCredentials(); }, { immediate: true });
</script>

<template>
  <div class="settings-body">
    <div class="settings-pane">
      <div class="grid storage-grid">
        <div class="card glass" id="dev-tools">
          <h3>
            {{ t("settings.dev.title") }}
            <span class="qx-badge qx-badge--danger">{{ t("settings.dev.modeBadge") }}</span>
          </h3>
          <div class="qx-hint qx-hint--red dev-mode-warning">
            <div class="dev-mode-warning__text">
              <strong>{{ t("settings.dev.warningTitle") }}</strong>
              <span>{{ t("settings.dev.warningBody") }}</span>
            </div>
          </div>
          <div class="settings-actions">
            <button class="qx-btn qx-btn--sm" @click="previewUpdate = true">{{ t("settings.dev.testUpdateModal") }}</button>
            <button class="qx-btn qx-btn--sm" @click="emit('reloadChangelog')">{{ t("settings.dev.reloadChangelog") }}</button>
            <button class="qx-btn qx-btn--sm qx-btn--danger" @click="emit('setDevMode', false)">{{ t("settings.dev.exit") }}</button>
          </div>
          <div class="appearance-divider"></div>
          <div class="choice-row">
            <div class="choice-info">
              <span class="choice-label">{{ t("settings.dev.titlebarThemeToggle") }}</span>
            </div>
            <button
              class="toggle"
              :class="{ on: titlebarThemeToggle }"
              role="switch"
              :aria-checked="titlebarThemeToggle"
              @click="titlebarThemeToggle = !titlebarThemeToggle"
            >
              <span class="knob"></span>
            </button>
          </div>
          <div class="choice-row">
            <div class="choice-info">
              <span class="choice-label">{{ t("settings.dev.allowDevTools") }}</span>
              <p class="choice-hint">{{ t("settings.dev.allowDevToolsHint") }}</p>
            </div>
            <button
              class="toggle"
              :class="{ on: allowDevTools }"
              role="switch"
              :aria-checked="allowDevTools"
              @click="setAllowDevTools(!allowDevTools)"
            >
              <span class="knob"></span>
            </button>
          </div>
          <div class="choice-row">
            <div class="choice-info">
              <span class="choice-label">{{ t("settings.dev.showFrp") }}</span>
              <p class="choice-hint">{{ t("settings.dev.showFrpHint") }}</p>
            </div>
            <button
              class="toggle"
              :class="{ on: frpEnabled }"
              role="switch"
              :aria-checked="frpEnabled"
              @click="setFrpEnabled(!frpEnabled)"
            >
              <span class="knob"></span>
            </button>
          </div>
        </div>

        <!-- 微软登录 OAuth Client ID（仅开发构建；正式包由 CI 注入） -->
        <div v-if="showMsClientId" class="card glass" id="dev-ms-client-id">
          <h3>{{ t("settings.dev.msCidTitle") }}</h3>
          <div class="dev-cred-row">
            <n-input v-model:value="msClientIdDraft" placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" />
            <button class="qx-btn qx-btn--sm qx-btn--primary" :disabled="msClientIdSaving" @click="saveMsClientId">
              {{ t("settings.dev.msCidSave") }}
            </button>
          </div>
          <p class="hint">{{ t("settings.dev.msCidHint") }}</p>
        </div>

        <!-- OAuth2 接入凭证（开发期测试用） -->
        <div class="card glass" id="dev-oauth">
          <h3>{{ t("settings.dev.oauthCredTitle") }}</h3>
          <p class="hint">{{ t("settings.dev.oauthCredHint") }}</p>
          <div class="dev-cred-group">
            <span class="choice-label">{{ t("settings.dev.lsClientId") }}</span>
            <div class="dev-cred-row">
              <n-input v-model:value="devLsClientId" :placeholder="t('settings.dev.lsClientIdPh')" />
              <button class="qx-btn qx-btn--sm qx-btn--primary" :disabled="devCredSaving" @click="saveDevLsCredentials">{{ t("settings.dev.credSave") }}</button>
            </div>
            <span class="choice-label">{{ t("settings.dev.lsClientSecret") }}</span>
            <div class="dev-cred-row">
              <n-input v-model:value="devLsClientSecret" type="password" show-password-on="click" :placeholder="t('settings.dev.lsSecretPh')" />
            </div>
            <p class="hint">
              <span v-if="accounts.yggOauthConfig?.configured" class="dev-cred-ok">{{ t("settings.dev.lsConfigured") }}</span>
              <span v-else class="dev-cred-no">{{ t("settings.dev.lsNotConfigured") }}</span>
              <span v-if="accounts.yggOauthConfig?.secretConfigured" class="dev-cred-ok"> · {{ t("settings.dev.lsSecretSet") }}</span>
              <span v-else class="dev-cred-no"> · {{ t("settings.dev.lsSecretUnset") }}</span>
            </p>
            <p class="hint">{{ t("settings.dev.lsRedirect") }}：<code class="mono">{{ accounts.yggOauthConfig?.redirectUri ?? "qookix://oauth/littleskin/callback" }}</code></p>
          </div>
        </div>
      </div>
    </div>
    <SettingsUpdateDialog v-model:show="previewUpdate" :offer="null" preview-version="9.9.9" />
  </div>
</template>
