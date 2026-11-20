<script setup lang="ts">
/**
 * QookiX 账号弹窗：登录 / 注册 / 找回密码 三态，全部在启动器内完成。
 *
 * 只有「点开邮件里的链接」这一步必须在浏览器做（验证邮箱 & 设置新密码），
 * 邮件以外的表单都不再跳外部页面。
 */
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { NInput, NModal, useMessage } from "naive-ui";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../api";
import { qookixErrorMessage, useQookixStore } from "../stores/qookix";
import type { QookixApiError, QookixRegisterPolicy } from "../types";

/** 用户协议（文档站托管，注册前必须勾选） */
const AGREEMENT_URL = "https://docs.qookix.cn/agreement.html";
/** 需要人机验证时的网页注册入口（Turnstile 在 Tauri WebView 里不可用） */
const REGISTER_WEB_URL = "https://service.qookix.cn/register.html";

type Mode = "login" | "register" | "twofa" | "forgot" | "verifySent" | "forgotSent";

const { t } = useI18n();
const store = useQookixStore();
const message = useMessage();

const show = ref(false);
const mode = ref<Mode>("login");
const busy = ref(false);
const error = ref("");
/** 登录失败的原始错误码：email_not_verified 时要给「重发验证邮件」的入口 */
const errorCode = ref("");
const sentTo = ref("");

const account = ref("");
const password = ref("");
const rUsername = ref("");
const rEmail = ref("");
const rPassword = ref("");
const rNickname = ref("");
const agreeTerms = ref(false);
const inviteCode = ref("");
const termsUrl = ref(AGREEMENT_URL);
const regPolicy = ref<QookixRegisterPolicy | null>(null);
const forgotLogin = ref("");
// 登录第二步
const twofaChallenge = ref("");
const twofaCode = ref("");

const title = computed(() =>
  mode.value === "register"
    ? t("qookix.registerTitle")
    : mode.value === "twofa"
      ? t("qookix.twofa.title")
      : mode.value === "forgot"
        ? t("qookix.forgotTitle")
        : t("qookix.title"),
);

function open() {
  show.value = true;
  mode.value = "login";
  error.value = "";
  errorCode.value = "";
  password.value = "";
}

defineExpose({ open });

function switchMode(next: Mode) {
  mode.value = next;
  error.value = "";
  errorCode.value = "";
  // 每次进入注册页都从"未同意"开始，避免上一次勾选被误当成这一次的同意
  if (next === "register") {
    agreeTerms.value = false;
    inviteCode.value = "";
    void loadRegisterPolicy();
  }
}

/** 注册策略决定要不要显示邀请码、以及协议链接的最终地址 */
async function loadRegisterPolicy() {
  try {
    const p = await api.qookixRegisterPolicy();
    regPolicy.value = p;
    if (p.terms_url) termsUrl.value = p.terms_url;
  } catch {
    // 拿不到策略就按默认表单走，服务端仍会拦
  }
}

function openAgreement() {
  openUrl(termsUrl.value || AGREEMENT_URL).catch(() => {});
}

function openWebRegister() {
  openUrl(REGISTER_WEB_URL).catch(() => {});
}

function fail(e: unknown) {
  error.value = qookixErrorMessage(e);
  const code = (e as QookixApiError | undefined)?.code;
  errorCode.value = typeof code === "string" ? code : "";
}

function validEmail(v: string) {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(v);
}

async function submitLogin() {
  if (busy.value) return;
  const login = account.value.trim();
  if (!login || !password.value) {
    error.value = t("qookix.err.invalidCredentials");
    errorCode.value = "";
    return;
  }
  error.value = "";
  errorCode.value = "";
  busy.value = true;
  try {
    const challenge = await store.login(login, password.value);
    if (challenge) {
      // 账号开了两步验证：留在弹窗里输入动态码
      twofaChallenge.value = challenge.challenge_token;
      twofaCode.value = "";
      mode.value = "twofa";
      return;
    }
    message.success(t("qookix.loginSuccess"));
    password.value = "";
    show.value = false;
  } catch (e) {
    fail(e);
  } finally {
    busy.value = false;
  }
}

async function submitTwoFA() {
  if (busy.value) return;
  const code = twofaCode.value.trim();
  error.value = "";
  errorCode.value = "";
  if (!code) {
    error.value = t("qookix.err.twoFactorCodeRequired");
    return;
  }
  busy.value = true;
  try {
    await store.login2fa(twofaChallenge.value, code);
    message.success(t("qookix.loginSuccess"));
    twofaCode.value = "";
    twofaChallenge.value = "";
    password.value = "";
    show.value = false;
  } catch (e) {
    fail(e);
  } finally {
    busy.value = false;
  }
}

async function submitRegister() {
  if (busy.value) return;
  const username = rUsername.value.trim();
  const email = rEmail.value.trim();
  error.value = "";
  errorCode.value = "";
  // 本地先挡一遍，省一次往返（服务端仍会再校验）
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]{2,23}$/.test(username)) {
    error.value = t("qookix.err.invalidUsername");
    return;
  }
  if (!validEmail(email)) {
    error.value = t("qookix.err.invalidEmail");
    return;
  }
  if (rPassword.value.length < 8 || rPassword.value.length > 72) {
    error.value = t("qookix.err.invalidPassword");
    return;
  }
  if (!agreeTerms.value) {
    error.value = t("qookix.err.termsRequired");
    return;
  }
  const invite = inviteCode.value.trim();
  if (regPolicy.value?.invite_required && !invite) {
    error.value = t("qookix.err.inviteRequired");
    return;
  }
  busy.value = true;
  try {
    const data = await store.register(
      username,
      email,
      rPassword.value,
      rNickname.value.trim() || undefined,
      invite || undefined,
      agreeTerms.value,
    );
    if (data.email_verification_required) {
      sentTo.value = email;
      mode.value = "verifySent";
      rPassword.value = "";
    } else {
      message.success(t("qookix.loginSuccess"));
      show.value = false;
    }
  } catch (e) {
    fail(e);
  } finally {
    busy.value = false;
  }
}

async function submitForgot() {
  if (busy.value) return;
  const login = forgotLogin.value.trim();
  if (!login) {
    error.value = t("qookix.err.invalidCredentials");
    return;
  }
  error.value = "";
  busy.value = true;
  try {
    await store.forgotPassword(login);
    sentTo.value = login;
    mode.value = "forgotSent";
  } catch (e) {
    fail(e);
  } finally {
    busy.value = false;
  }
}

/** 验证邮件没收到时重发；登录报 email_not_verified 也走这里 */
async function resendVerification() {
  const login = sentTo.value || rEmail.value.trim() || account.value.trim();
  if (!login) return;
  try {
    await store.resendVerification(login);
    message.success(t("qookix.resendSent"));
  } catch (e) {
    message.error(qookixErrorMessage(e));
  }
}

function goForgot() {
  forgotLogin.value = account.value.includes("@") ? account.value.trim() : "";
  switchMode("forgot");
}
</script>

<template>
  <n-modal
    v-model:show="show"
    preset="card"
    :title="title"
    style="width: 420px; max-width: 92vw"
    :bordered="false"
  >
    <div class="qx-box">
      <!-- 登录 -->
      <template v-if="mode === 'login'">
        <p class="qx-intro">{{ t("qookix.intro") }}</p>

        <label class="qx-field">
          <span class="qx-label">{{ t("qookix.accountPlaceholder") }}</span>
          <n-input
            v-model:value="account"
            :placeholder="t('qookix.accountPlaceholder')"
            :disabled="busy"
            @keyup.enter="submitLogin"
          />
        </label>

        <label class="qx-field">
          <span class="qx-label">{{ t("qookix.passwordPlaceholder") }}</span>
          <n-input
            v-model:value="password"
            type="password"
            show-password-on="click"
            :placeholder="t('qookix.passwordPlaceholder')"
            :disabled="busy"
            @keyup.enter="submitLogin"
          />
        </label>

        <p v-if="error" class="qx-error">{{ error }}</p>
        <button v-if="errorCode === 'email_not_verified'" class="qx-inline-link" @click="resendVerification">
          {{ t("qookix.resendVerify") }}
        </button>

        <button class="qx-submit" :disabled="busy" @click="submitLogin">
          {{ busy ? t("qookix.loggingIn") : t("qookix.login") }}
        </button>

        <div class="qx-links">
          <button class="qx-link" @click="switchMode('register')">{{ t("qookix.noAccount") }}</button>
          <span class="qx-dot">·</span>
          <button class="qx-link" @click="goForgot">{{ t("qookix.forgot") }}</button>
          <span class="qx-dot">·</span>
          <button class="qx-link" @click="show = false">{{ t("qookix.later") }}</button>
        </div>
      </template>

      <!-- 登录第二步：两步验证 -->
      <template v-else-if="mode === 'twofa'">
        <p class="qx-intro">{{ t("qookix.twofa.intro") }}</p>

        <label class="qx-field">
          <span class="qx-label">{{ t("qookix.twofa.codeLabel") }}</span>
          <n-input
            v-model:value="twofaCode"
            :placeholder="t('qookix.twofa.codePlaceholder')"
            :disabled="busy"
            maxlength="16"
            @keyup.enter="submitTwoFA"
          />
        </label>

        <p v-if="error" class="qx-error">{{ error }}</p>

        <button class="qx-submit" :disabled="busy" @click="submitTwoFA">
          {{ busy ? t("qookix.loggingIn") : t("qookix.twofa.submit") }}
        </button>

        <div class="qx-links">
          <button class="qx-link" @click="switchMode('login')">{{ t("qookix.backToLogin") }}</button>
          <span class="qx-dot">·</span>
          <button class="qx-link" @click="show = false">{{ t("qookix.later") }}</button>
        </div>
      </template>

      <!-- 注册 -->
      <template v-else-if="mode === 'register'">
        <p class="qx-intro">{{ t("qookix.registerIntro") }}</p>

        <label class="qx-field">
          <span class="qx-label">{{ t("qookix.usernamePlaceholder") }}</span>
          <n-input
            v-model:value="rUsername"
            :placeholder="t('qookix.usernamePlaceholder')"
            maxlength="24"
            :disabled="busy"
            @keyup.enter="submitRegister"
          />
        </label>

        <label class="qx-field">
          <span class="qx-label">{{ t("qookix.emailPlaceholder") }}</span>
          <n-input
            v-model:value="rEmail"
            :placeholder="t('qookix.emailPlaceholder')"
            :disabled="busy"
            @keyup.enter="submitRegister"
          />
        </label>

        <label class="qx-field">
          <span class="qx-label">{{ t("qookix.passwordHint") }}</span>
          <n-input
            v-model:value="rPassword"
            type="password"
            show-password-on="click"
            :placeholder="t('qookix.passwordHint')"
            :disabled="busy"
            @keyup.enter="submitRegister"
          />
        </label>

        <label class="qx-field">
          <span class="qx-label">{{ t("qookix.nicknamePlaceholder") }}</span>
          <n-input
            v-model:value="rNickname"
            :placeholder="t('qookix.nicknamePlaceholder')"
            maxlength="24"
            :disabled="busy"
            @keyup.enter="submitRegister"
          />
        </label>

        <label v-if="regPolicy?.invite_required" class="qx-field">
          <span class="qx-label">{{ t("qookix.inviteCodeLabel") }}</span>
          <n-input
            v-model:value="inviteCode"
            :placeholder="t('qookix.inviteCodePlaceholder')"
            :disabled="busy"
            maxlength="16"
            @keyup.enter="submitRegister"
          />
        </label>

        <p v-if="regPolicy?.turnstile_enabled" class="qx-intro">
          {{ t("qookix.turnstileWebHint") }}
        </p>

        <label class="qx-terms">
          <input v-model="agreeTerms" type="checkbox" :disabled="busy" />
          <span>
            {{ t("qookix.termsPrefix") }}
            <button type="button" class="qx-terms-link" @click="openAgreement">
              {{ t("qookix.termsLink") }}
            </button>
          </span>
        </label>

        <p v-if="error" class="qx-error">{{ error }}</p>

        <button
          v-if="!regPolicy?.turnstile_enabled"
          class="qx-submit"
          :disabled="busy"
          @click="submitRegister"
        >
          {{ busy ? t("qookix.registering") : t("qookix.registerSubmit") }}
        </button>
        <button v-else class="qx-submit" @click="openWebRegister">
          {{ t("qookix.openWebRegister") }}
        </button>

        <div class="qx-links">
          <button class="qx-link" @click="switchMode('login')">{{ t("qookix.haveAccount") }}</button>
          <span class="qx-dot">·</span>
          <button class="qx-link" @click="show = false">{{ t("qookix.later") }}</button>
        </div>
      </template>

      <!-- 找回密码 -->
      <template v-else-if="mode === 'forgot'">
        <p class="qx-intro">{{ t("qookix.forgotIntro") }}</p>

        <label class="qx-field">
          <span class="qx-label">{{ t("qookix.accountPlaceholder") }}</span>
          <n-input
            v-model:value="forgotLogin"
            :placeholder="t('qookix.accountPlaceholder')"
            :disabled="busy"
            @keyup.enter="submitForgot"
          />
        </label>

        <p v-if="error" class="qx-error">{{ error }}</p>

        <button class="qx-submit" :disabled="busy" @click="submitForgot">
          {{ busy ? t("qookix.sending") : t("qookix.forgotSubmit") }}
        </button>

        <div class="qx-links">
          <button class="qx-link" @click="switchMode('login')">{{ t("qookix.backToLogin") }}</button>
        </div>
      </template>

      <!-- 验证邮件已发送 -->
      <template v-else-if="mode === 'verifySent'">
        <p class="qx-sent-title">{{ t("qookix.verifySentTitle") }}</p>
        <p class="qx-intro">{{ t("qookix.verifySentBody", { email: sentTo }) }}</p>
        <button class="qx-submit" @click="switchMode('login')">{{ t("qookix.backToLogin") }}</button>
        <div class="qx-links">
          <button class="qx-link" @click="resendVerification">{{ t("qookix.resendVerify") }}</button>
          <span class="qx-dot">·</span>
          <button class="qx-link" @click="show = false">{{ t("qookix.close") }}</button>
        </div>
      </template>

      <!-- 重置邮件已发送 -->
      <template v-else>
        <p class="qx-sent-title">{{ t("qookix.forgotTitle") }}</p>
        <p class="qx-intro">{{ t("qookix.resetSent") }}</p>
        <button class="qx-submit" @click="switchMode('login')">{{ t("qookix.backToLogin") }}</button>
        <div class="qx-links">
          <button class="qx-link" @click="show = false">{{ t("qookix.close") }}</button>
        </div>
      </template>
    </div>
  </n-modal>
</template>

<style scoped>
.qx-box {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.qx-intro {
  margin: 0;
  font-size: var(--fs-xs);
  line-height: 1.6;
  color: var(--text-3);
}
.qx-sent-title {
  margin: 0;
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text-1);
}
.qx-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.qx-label {
  font-size: var(--fs-xs);
  color: var(--text-2);
}
.qx-submit {
  height: 36px;
  border: none;
  border-radius: var(--r-md);
  background: var(--accent);
  /* 跟随全局主题对比色，避免自定义主题色时文字发黑 */
  color: var(--accent-contrast, #fff);
  font-size: var(--fs-sm);
  font-weight: 600;
  cursor: pointer;
  transition: transform var(--dur-sm, 0.15s) var(--ease-out, ease),
    filter var(--dur-sm, 0.15s) var(--ease-out, ease);
}
.qx-submit:hover:not(:disabled) {
  filter: brightness(1.06);
}
.qx-submit:active:not(:disabled) {
  transform: scale(0.97);
}
.qx-submit:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
.qx-terms {
  display: flex;
  align-items: flex-start;
  gap: 7px;
  font-size: var(--fs-xs);
  line-height: 1.6;
  color: var(--text-3);
  cursor: pointer;
}
.qx-terms input {
  margin-top: 2px;
  accent-color: var(--accent);
  cursor: pointer;
}
.qx-terms-link {
  border: none;
  background: none;
  padding: 0;
  font-size: inherit;
  color: var(--accent);
  cursor: pointer;
}
.qx-terms-link:hover {
  text-decoration: underline;
}
.qx-error {
  margin: 0;
  font-size: var(--fs-xs);
  line-height: 1.5;
  color: var(--danger, #e5534b);
}
.qx-inline-link {
  align-self: flex-start;
  border: none;
  background: none;
  padding: 0;
  margin-top: -6px;
  font-size: var(--fs-xs);
  color: var(--accent);
  cursor: pointer;
}
.qx-inline-link:hover {
  text-decoration: underline;
}
.qx-links {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.qx-link {
  border: none;
  background: none;
  padding: 0;
  font-size: var(--fs-xs);
  color: var(--text-2);
  cursor: pointer;
  transition: color var(--dur-sm, 0.15s) var(--ease-out, ease);
}
.qx-link:hover {
  color: var(--accent);
}
.qx-dot {
  color: var(--text-3);
}
</style>
