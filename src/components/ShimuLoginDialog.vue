<script setup lang="ts">
/**
 * 拾木空间账号弹窗：OAuth 授权登录（浏览器） + 用户资料展示。
 *
 * 授权完成后站点会跳回 `qookix://oauth/callback`，由 deepLink 组合式函数
 * 交给 store 换 token；在开发环境（协议未注册）下也可以把回调地址整段
 * 粘进「手动完成」输入框，效果相同。
 */
import { computed, ref, watch } from "vue";
import { NButton, NInput, NModal, useMessage } from "naive-ui";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useI18n } from "vue-i18n";
import { shimuErrorMessage, useShimuStore } from "../stores/shimu";

const emit = defineEmits<{ loggedIn: [] }>();
const { t } = useI18n();
const message = useMessage();
const store = useShimuStore();

const show = ref(false);
/** 已确认第三方平台声明（每次打开弹窗都要重新确认） */
const agreed = ref(false);
const advanced = ref(false);
const clientIdInput = ref("");
const clientSecretInput = ref("");
const savingCreds = ref(false);
/** 浏览器授权页已打开，正在等回调 */
const waiting = ref(false);
const manualInput = ref("");
const busy = ref(false);

const redirectUri = computed(() => store.config?.redirectUri ?? "qookix://oauth/callback");
/** 只有开发构建才显示「接入设置」（正式包的凭证靠打包时注入） */
const devMode = computed(() => store.config?.devMode === true);
/** 正式包且未内置凭证：功能不可用，不给用户填凭证的入口 */
const unavailable = computed(() => !store.config?.configured && !devMode.value);
const expireText = computed(() => {
  if (!store.status.expiresAt) return "";
  const d = new Date(store.status.expiresAt * 1000);
  return d.toLocaleString();
});
const balanceText = computed(() => {
  const u = store.user;
  if (!u) return "";
  const parts: string[] = [];
  if (typeof u.score === "number") parts.push(t("shimu.score", { n: u.score }));
  if (typeof u.balance === "number") parts.push(t("shimu.balance", { n: u.balance.toFixed(2) }));
  const member = store.memberLabel;
  if (member) parts.push(member);
  return parts.join(" · ");
});

function open() {
  show.value = true;
  void store.init();
  clientIdInput.value = store.config?.clientId ?? "";
  clientSecretInput.value = "";
  manualInput.value = "";
  waiting.value = false;
  // 未登录时先展示第三方平台声明，确认后才继续登录
  agreed.value = false;
  // 开发构建且还没配凭证时才展开接入设置
  advanced.value = false;
}
defineExpose({ open });

watch(show, (v) => {
  if (!v) return;
  if (devMode.value && !store.configured) advanced.value = true;
});

// 深链回调换 token 成功后自动收起弹窗
watch(
  () => store.loggedIn,
  (v) => {
    if (!v) return;
    waiting.value = false;
    if (show.value) {
      show.value = false;
      message.success(t("shimu.loginSuccess"));
      emit("loggedIn");
    }
  }
);

async function saveCreds() {
  if (!clientIdInput.value.trim()) {
    message.warning(t("shimu.fillClientId"));
    return;
  }
  savingCreds.value = true;
  try {
    await store.setCredentials(clientIdInput.value.trim(), clientSecretInput.value.trim() || undefined);
    clientSecretInput.value = "";
    message.success(t("shimu.credsSaved"));
    advanced.value = false;
  } catch (e) {
    message.error(shimuErrorMessage(e));
  } finally {
    savingCreds.value = false;
  }
}

async function startLogin() {
  if (!store.configured) {
    // 开发构建：展开面板引导填写；正式包：只提示功能未启用
    if (devMode.value) {
      message.warning(t("shimu.needClientId"));
      advanced.value = true;
    } else {
      message.warning(t("shimu.notConfigured"));
    }
    return;
  }
  busy.value = true;
  try {
    const url = await store.startAuth();
    await openUrl(url);
    waiting.value = true;
  } catch (e) {
    message.error(shimuErrorMessage(e));
  } finally {
    busy.value = false;
  }
}

async function submitManual() {
  if (!manualInput.value.trim()) return;
  busy.value = true;
  try {
    const r = await store.handleCallback(manualInput.value);
    if (r === "denied") {
      message.warning(t("shimu.denied"));
      return;
    }
    manualInput.value = "";
    show.value = false;
    message.success(t("shimu.loginSuccess"));
    emit("loggedIn");
  } catch (e) {
    message.error(shimuErrorMessage(e));
  } finally {
    busy.value = false;
  }
}

async function logout() {
  try {
    await store.logout();
    message.success(t("shimu.loggedOut"));
  } catch (e) {
    message.error(shimuErrorMessage(e));
  }
}

function openSite() {
  const host = store.config?.host ?? "https://www.mcimg.com";
  openUrl(host).catch((e: unknown) => message.error(String(e)));
}

function copyRedirectUri() {
  navigator.clipboard
    ?.writeText(redirectUri.value)
    .then(() => message.success(t("shimu.redirectCopied")))
    .catch(() => {});
}
</script>

<template>
  <n-modal
    v-model:show="show"
    preset="card"
    :title="t('shimu.title')"
    style="width: 460px; max-width: 92vw"
  >
    <div class="shimu-box">
      <!-- 已登录：资料卡 -->
      <template v-if="store.loggedIn && store.user">
        <div class="shimu-user">
          <img v-if="store.avatarUrl" class="shimu-avatar" :src="store.avatarUrl" alt="" />
          <div v-else class="shimu-avatar shimu-avatar-ph">{{ store.nickname.slice(0, 1) }}</div>
          <div class="shimu-user-info">
            <div class="shimu-name">{{ store.nickname }}</div>
            <div class="shimu-meta">{{ balanceText }}</div>
          </div>
        </div>
        <p v-if="expireText" class="shimu-hint">{{ t("shimu.expiresAt", { time: expireText }) }}</p>
        <p class="shimu-hint">{{ t("shimu.loggedInHint") }}</p>
        <div class="shimu-row">
          <n-button size="small" @click="openSite">{{ t("shimu.openSite") }}</n-button>
          <n-button size="small" @click="startLogin" :loading="busy">{{ t("shimu.reauth") }}</n-button>
          <n-button size="small" tertiary type="error" @click="logout">{{ t("shimu.logout") }}</n-button>
        </div>
      </template>

      <!-- 未登录：先展示第三方平台声明，确认后再继续 -->
      <template v-else-if="!agreed">
        <p class="shimu-notice-title">{{ t("shimu.noticeTitle") }}</p>
        <div class="shimu-notice">
          <p>{{ t("shimu.noticePlatform") }}</p>
          <p>{{ t("shimu.noticeTrade") }}</p>
          <p>{{ t("shimu.noticeClient") }}</p>
        </div>
        <div class="shimu-row">
          <n-button size="small" @click="show = false">{{ t("common.cancel") }}</n-button>
          <n-button size="small" type="primary" class="grow" @click="agreed = true">
            {{ t("shimu.agreeContinue") }}
          </n-button>
        </div>
      </template>

      <!-- 未登录（已确认声明） -->
      <template v-else>
        <p class="shimu-hint">{{ t("shimu.intro") }}</p>
        <p v-if="store.status.expired" class="shimu-warn">{{ t("shimu.errExpired") }}</p>

        <!-- 正式包且未内置凭证：没有可用的授权入口，也不暴露接入设置 -->
        <p v-if="unavailable" class="shimu-warn">{{ t("shimu.notConfigured") }}</p>

        <template v-else>
          <template v-if="waiting">
            <p class="shimu-waiting">{{ t("shimu.waiting") }}</p>
            <div class="shimu-row">
              <n-button size="small" @click="waiting = false">{{ t("common.cancel") }}</n-button>
            </div>
          </template>
          <template v-else>
            <n-button type="primary" block :loading="busy" @click="startLogin">
              {{ t("shimu.login") }}
            </n-button>
          </template>

          <!-- 手动完成：仅开发构建可见（本地未注册 qookix:// 协议时用） -->
          <div v-if="devMode" class="shimu-manual">
            <n-input
              v-model:value="manualInput"
              size="small"
              :placeholder="t('shimu.manualPlaceholder')"
              @keyup.enter="submitManual"
            />
            <n-button size="small" :loading="busy" :disabled="!manualInput.trim()" @click="submitManual">
              {{ t("shimu.manualDone") }}
            </n-button>
          </div>

          <!-- 接入设置：仅开发构建可见 -->
          <template v-if="devMode">
            <button class="shimu-toggle" @click="advanced = !advanced">
              {{ advanced ? "▾" : "▸" }} {{ t("shimu.advanced") }}
            </button>
            <div v-if="advanced" class="shimu-advanced">
              <n-input
                v-model:value="clientIdInput"
                size="small"
                :placeholder="t('shimu.clientIdPlaceholder')"
              />
              <n-input
                v-model:value="clientSecretInput"
                size="small"
                type="password"
                show-password-on="click"
                :placeholder="
                  store.config?.secretConfigured ? t('shimu.secretKeep') : t('shimu.clientSecretPlaceholder')
                "
              />
              <n-button size="small" :loading="savingCreds" @click="saveCreds">
                {{ t("shimu.saveCreds") }}
              </n-button>
              <p class="shimu-hint">{{ t("shimu.secretHint") }}</p>
              <p class="shimu-hint">
                {{ t("shimu.redirectHint") }}
                <a class="shimu-link" @click.prevent="copyRedirectUri">{{ redirectUri }}</a>
              </p>
            </div>
          </template>
        </template>
      </template>
    </div>
  </n-modal>
</template>

<style scoped>
.shimu-box {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.shimu-hint {
  margin: 0;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-3);
}
/* 第三方平台声明：登录前必须确认 */
.shimu-notice-title {
  margin: 0;
  font-size: 13px;
  color: var(--text-1);
}
.shimu-notice {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px solid var(--border);
  border-left: 3px solid var(--accent-35);
  background: var(--panel);
}
.shimu-notice p {
  margin: 0;
  font-size: 12px;
  line-height: 1.7;
  color: var(--text-2);
}
/* naive-ui 的 block 属性是 width:100%，放在 flex 行里会和左边的按钮相加溢出卡片，
   所以改用 flex:1 占满剩余宽度 */
.shimu-row .grow { flex: 1; min-width: 0; }
.shimu-warn {
  margin: 0;
  font-size: 12px;
  color: var(--accent);
}
.shimu-waiting {
  margin: 0;
  font-size: 13px;
  color: var(--text-1);
}
.shimu-user {
  display: flex;
  align-items: center;
  gap: 12px;
}
.shimu-avatar {
  width: 44px;
  height: 44px;
  border-radius: 12px;
  object-fit: cover;
  flex: none;
  background: var(--w-04);
}
.shimu-avatar-ph {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 18px;
  color: var(--text-3);
}
.shimu-user-info {
  min-width: 0;
}
.shimu-name {
  font-size: 14px;
  color: var(--text-1);
}
.shimu-meta {
  font-size: 12px;
  color: var(--text-3);
}
.shimu-row {
  display: flex;
  gap: 8px;
}
.shimu-manual {
  display: flex;
  gap: 8px;
  align-items: center;
}
.shimu-toggle {
  align-self: flex-start;
  padding: 2px 0;
  border: none;
  background: none;
  color: var(--accent);
  font-family: inherit;
  font-size: 12px;
  cursor: pointer;
}
.shimu-advanced {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.shimu-advanced .shimu-hint {
  word-break: break-all;
}
.shimu-link {
  color: var(--accent);
  cursor: pointer;
  font-family: "Cascadia Code", Consolas, monospace;
}
</style>
