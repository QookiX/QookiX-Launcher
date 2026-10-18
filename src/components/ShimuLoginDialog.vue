<script setup lang="ts">
/**
 * 拾木空间账号弹窗：OAuth 授权登录（浏览器） + 用户资料展示。
 *
 * 授权完成后站点会跳回 `qookix://oauth/callback`，由 deepLink 组合式函数
 * 交给 store 换 token；在开发环境（协议未注册）下也可以把回调地址整段
 * 粘进「手动完成」输入框，效果相同。
 */
import { computed, ref, watch } from "vue";
import { NButton, NModal, useMessage } from "naive-ui";
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
/** 浏览器授权页已打开，正在等回调 */
const waiting = ref(false);
const busy = ref(false);

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
  waiting.value = false;
  // 未登录时先展示第三方平台声明，确认后才继续登录
  agreed.value = false;
}
defineExpose({ open });

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

async function startLogin() {
  if (!store.configured) {
    message.warning(t("shimu.notConfigured"));
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
</style>
