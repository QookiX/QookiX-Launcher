<script setup lang="ts">
import { computed, onActivated, onMounted, ref, watch } from "vue";
import { useFavoritesStore } from "../../stores/favorites";
import { NModal, NSelect, NInput, useMessage, useDialog } from "naive-ui";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useSettingsStore } from "../../stores/settings";
import { useAccountsStore } from "../../stores/accounts";
import { api } from "../../api";
import { IconExternal, IconGithub, IconGlobe, IconRefresh } from "../../components/icons";
import QRCode from "qrcode";
import type { QookixTwoFactorStatus, QookixTwoFactorSetup } from "../../types";
import ShimuLoginDialog from "../../components/ShimuLoginDialog.vue";
import QookixLoginDialog from "../../components/QookixLoginDialog.vue";
import FrpDialog from "../../components/FrpDialog.vue";
import { useSkinRenderer } from "../../composables/useSkinRenderer";
import { useI18n } from "vue-i18n";
import { shimuErrorMessage, useShimuStore } from "../../stores/shimu";
import { qookixErrorMessage, useQookixStore } from "../../stores/qookix";
import { showFrpFeatures } from "../../composables/useDevFlags";

const settings = useSettingsStore();
const { t } = useI18n();
const message = useMessage();
const dialog = useDialog();
const accounts = useAccountsStore();
const shimuStore = useShimuStore();
const qookixStore = useQookixStore();
const props = defineProps<{ cloudOpen: boolean }>();
const emit = defineEmits<{ openCloud: [] }>();

/** GitHub 账户（云存档与时光机共用同一套授权） */
const cloudAccount = ref("");
const cloudRepo = ref("");
/**
 * 第三方账号（目前只有拾木空间）：账号本身由 stores/shimu 维护，
 * 这里只做展示 + 唤起授权弹窗，登录 / 退出后 store 会自己刷新登录态。
 */
const shimuLoginRef = ref<InstanceType<typeof ShimuLoginDialog> | null>(null);
const shimuRefreshing = ref(false);
const shimuAvatar = computed(() => shimuStore.avatarUrl);
/** 有令牌但已过期：登录态算「未登录」，界面要单独提示重新授权 */
const shimuExpired = computed(() => shimuStore.status.expired && !shimuStore.loggedIn);
const shimuBadge = computed<"on" | "off" | "expired">(() =>
  shimuStore.loggedIn ? "on" : shimuExpired.value ? "expired" : "off"
);
const shimuBadgeLabel = computed(() =>
  shimuBadge.value === "on"
    ? t("settings.accounts.statusOn")
    : shimuBadge.value === "expired"
      ? t("settings.accounts.statusExpired")
      : t("settings.accounts.statusOff")
);
const shimuMetaText = computed(() => {
  if (!shimuStore.loggedIn) return t("settings.accounts.guestHint");
  const parts: string[] = [];
  if (shimuStore.nickname) parts.push(shimuStore.nickname);
  const u = shimuStore.user;
  if (typeof u?.score === "number") parts.push(t("shimu.score", { n: u.score }));
  if (typeof u?.balance === "number") parts.push(t("shimu.balance", { n: u.balance.toFixed(2) }));
  if (shimuStore.memberLabel) parts.push(shimuStore.memberLabel);
  return parts.join(" · ");
});

function openShimuLogin() {
  shimuLoginRef.value?.open();
}

/**
 * QookiX 账号（可选）：只承载好友与联机房间目录，与启动账号完全无关。
 */
const frpRef = ref<InstanceType<typeof FrpDialog> | null>(null);
function openFrp() {
  frpRef.value?.open();
}

const qookixLoginRef = ref<InstanceType<typeof QookixLoginDialog> | null>(null);
const qookixBadge = computed<"on" | "off">(() => (qookixStore.loggedIn ? "on" : "off"));
const qookixBadgeLabel = computed(() =>
  qookixStore.loggedIn ? t("qookix.onlineBadge") : t("qookix.offlineBadge"),
);
const qookixMetaText = computed(() => {
  const u = qookixStore.user;
  if (!u) return t("qookix.intro");
  const parts = [`@${u.username}`];
  if (u.email) parts.push(u.email);
  if (u.mc_name) {
    parts.push(
      u.mc_verified
        ? t("qookix.mcNameVerified", { name: u.mc_name })
        : t("qookix.mcNameUnverified", { name: u.mc_name }),
    );
  }
  return parts.join(" · ");
});

const qookixMcVerified = computed(() => !!qookixStore.user?.mc_verified);
const qookixMcName = computed(() => qookixStore.user?.mc_name ?? "");
/**
 * 用来绑定的正版账号：优先当前选中的，否则取列表里第一个微软账号。
 * 不这么兜底的话，用户切到离线账号后按钮会直接消失，看着像功能没了。
 */
const bindTargetAccount = computed(() => {
  const cur = accounts.current;
  if (cur && cur.type === "microsoft") return cur;
  return accounts.accounts.find((a) => a.type === "microsoft") ?? null;
});
const canBindMc = computed(() => !!bindTargetAccount.value && !qookixMcVerified.value);

async function bindCurrentMc() {
  const acc = bindTargetAccount.value;
  if (!acc) {
    message.warning(t("qookix.err.mcOfflineAccount"));
    return;
  }
  try {
    const r = await qookixStore.bindMc(acc.uuid);
    const bound = (r?.user as { mc_name?: string } | undefined)?.mc_name || acc.username;
    message.success(t("qookix.bindMcDone", { name: bound }));
  } catch (e) {
    message.error(qookixErrorMessage(e));
  }
}

/** 解绑是有代价的（24 小时冷却 + 掉大厅权限），必须二次确认 */
function unbindMc() {
  dialog.warning({
    title: t("qookix.unbindTitle"),
    content: t("qookix.unbindConfirm"),
    positiveText: t("common.confirm"),
    negativeText: t("common.cancel"),
    onPositiveClick: async () => {
      try {
        const r = await qookixStore.unbindMc();
        const hours = Math.round((r?.retry_after ?? 86400) / 3600);
        message.success(t("qookix.unbindMcDone", { hours }));
      } catch (e) {
        message.error(qookixErrorMessage(e));
      }
    },
  });
}

// ---- 修改邮箱 ----
const emailOpen = ref(false);
const emailDraft = ref("");
const emailBusy = ref(false);

function openEmailChange() {
  emailDraft.value = "";
  emailOpen.value = true;
}

async function submitEmailChange() {
  const v = emailDraft.value.trim();
  if (!v || !v.includes("@")) {
    message.warning(t("qookix.err.invalidEmail"));
    return;
  }
  emailBusy.value = true;
  try {
    await qookixStore.changeEmail(v);
    emailOpen.value = false;
    message.success(t("qookix.emailSent"));
  } catch (e) {
    message.error(qookixErrorMessage(e));
  } finally {
    emailBusy.value = false;
  }
}

function openQookixLogin() {
  qookixLoginRef.value?.open();
}

// ---- 修改密码 ----
const pwdOpen = ref(false);
const pwdOld = ref("");
const pwdNew = ref("");
const pwdNew2 = ref("");
const pwdBusy = ref(false);

function openChangePassword() {
  pwdOld.value = "";
  pwdNew.value = "";
  pwdNew2.value = "";
  pwdOpen.value = true;
}

async function submitChangePassword() {
  if (pwdBusy.value) return;
  if (pwdNew.value.length < 8 || pwdNew.value.length > 72) {
    message.warning(t("qookix.password.tooShort"));
    return;
  }
  if (pwdNew.value !== pwdNew2.value) {
    message.warning(t("qookix.password.mismatch"));
    return;
  }
  pwdBusy.value = true;
  try {
    await api.qookixChangePassword(pwdOld.value, pwdNew.value);
    pwdOpen.value = false;
    message.success(t("qookix.password.success"));
  } catch (e) {
    message.error(qookixErrorMessage(e));
  } finally {
    pwdBusy.value = false;
  }
}

// ---- 两步验证 ----
const tfaOpen = ref(false);
const tfaLoading = ref(false);
const tfaBusy = ref(false);
const tfaStatus = ref<QookixTwoFactorStatus | null>(null);
const tfaSetup = ref<QookixTwoFactorSetup | null>(null);
const tfaQr = ref("");
const tfaCode = ref("");
const tfaPassword = ref("");
const tfaRecovery = ref<string[]>([]);
const tfaEnabled = computed(() => !!tfaStatus.value?.enabled);

async function openTwoFactor() {
  tfaOpen.value = true;
  tfaLoading.value = true;
  tfaBusy.value = false;
  tfaSetup.value = null;
  tfaQr.value = "";
  tfaCode.value = "";
  tfaPassword.value = "";
  tfaRecovery.value = [];
  try {
    const s = await api.qookix2faStatus();
    tfaStatus.value = s;
    if (!s.enabled) {
      // 未开启：直接生成待启用的种子和恢复码
      const setup = await api.qookix2faSetup();
      tfaSetup.value = setup;
      tfaRecovery.value = setup.recovery_codes ?? [];
      try {
        tfaQr.value = await QRCode.toDataURL(setup.otpauth_uri, { margin: 1, width: 220 });
      } catch {
        tfaQr.value = "";
      }
    }
  } catch (e) {
    message.error(qookixErrorMessage(e));
    tfaOpen.value = false;
  } finally {
    tfaLoading.value = false;
  }
}

async function submit2faEnable() {
  if (tfaBusy.value) return;
  const code = tfaCode.value.trim();
  if (!code) {
    message.warning(t("qookix.err.twoFactorCodeRequired"));
    return;
  }
  tfaBusy.value = true;
  try {
    await api.qookix2faEnable(code);
    message.success(t("qookix.twofa.enabledToast"));
    tfaOpen.value = false;
    void refresh2faStatus();
  } catch (e) {
    message.error(qookixErrorMessage(e));
  } finally {
    tfaBusy.value = false;
  }
}

async function submit2faDisable() {
  if (tfaBusy.value) return;
  if (!tfaPassword.value || !tfaCode.value.trim()) {
    message.warning(t("qookix.err.twoFactorCodeRequired"));
    return;
  }
  tfaBusy.value = true;
  try {
    await api.qookix2faDisable(tfaPassword.value, tfaCode.value.trim());
    message.success(t("qookix.twofa.disabledToast"));
    tfaOpen.value = false;
    void refresh2faStatus();
  } catch (e) {
    message.error(qookixErrorMessage(e));
  } finally {
    tfaBusy.value = false;
  }
}

async function refresh2faStatus() {
  if (!qookixStore.loggedIn) {
    tfaStatus.value = null;
    return;
  }
  try {
    tfaStatus.value = await api.qookix2faStatus();
  } catch {
    /* 状态拿不到不影响账号其他功能 */
  }
}

async function copyRecoveryCodes() {
  if (!tfaRecovery.value.length) return;
  try {
    await navigator.clipboard.writeText(tfaRecovery.value.join("\n"));
    message.success(t("qookix.twofa.copied"));
  } catch {
    /* 剪贴板不可用时不提示 */
  }
}

// 登录状态变化时补一次 2FA 状态；退出登录清掉
watch(
  () => qookixStore.loggedIn,
  (v) => {
    if (v) void refresh2faStatus();
    else tfaStatus.value = null;
  },
  { immediate: true },
);

// ---- 我的皮肤（QookiX 账号自带托管，复用 SkinView 的渲染器）----
const skinCanvas = ref<HTMLCanvasElement | null>(null);
const skinFileRef = ref<HTMLInputElement | null>(null);
const skinBusy = ref(false);
const skinModel = ref<"classic" | "slim">("classic");
const skinImage = ref("");
const skinStatus = ref("approved");
const skinReason = ref("");
const skinRenderer = useSkinRenderer(skinCanvas);
const qookixNickStatus = computed(() => qookixStore.user?.nickname_status ?? "approved");
const qookixNickReason = computed(() => qookixStore.user?.nickname_review_reason ?? "");
// 登录状态变化时补拉一次皮肤（onMounted 时可能还没登录）
watch(
  () => qookixStore.loggedIn,
  (v) => {
    if (v) void loadMySkin();
  },
  { immediate: true },
);
const skinModelOptions = computed(() => [
  { label: t("qookix.skin.classic"), value: "classic" },
  { label: t("qookix.skin.slim"), value: "slim" },
]);

async function loadMySkin() {
  if (!qookixStore.loggedIn) return;
  try {
    const s = await api.qookixMySkin();
    skinImage.value = s.skin || "";
    skinStatus.value = s.status || "approved";
    skinReason.value = s.reason || "";
    skinModel.value = s.model === "slim" ? "slim" : "classic";
    if (skinImage.value) {
      await skinRenderer.loadSkinFromSrc(skinImage.value);
      await skinRenderer.setModel(skinModel.value === "slim" ? "slim" : "default");
    }
  } catch {
    // 没登录 / 没皮肤都当作「没有」，不打扰用户
  }
}

function pickSkin() {
  skinFileRef.value?.click();
}

async function onSkinFile(e: Event) {
  const input = e.target as HTMLInputElement;
  const file = input.files?.[0];
  // 清空 value，否则选同一个文件不会再触发 change
  input.value = "";
  if (!file) return;
  if (!file.name.toLowerCase().endsWith(".png")) {
    message.warning(t("qookix.skin.hint"));
    return;
  }
  const dataUrl = await new Promise<string>((resolve, reject) => {
    const fr = new FileReader();
    fr.onload = () => resolve(String(fr.result ?? ""));
    fr.onerror = () => reject(fr.error);
    fr.readAsDataURL(file);
  });
  skinBusy.value = true;
  try {
    const s = await api.qookixUploadSkin(dataUrl, undefined, skinModel.value);
    skinImage.value = s.skin || "";
    skinStatus.value = s.status || "pending";
    skinReason.value = s.reason || "";
    if (skinImage.value) {
      await skinRenderer.loadSkinFromSrc(skinImage.value);
      await skinRenderer.setModel(skinModel.value === "slim" ? "slim" : "default");
    }
    message.success(t("qookix.skin.title"));
  } catch (err) {
    message.error(String(err));
  } finally {
    skinBusy.value = false;
  }
}

async function clearSkin() {
  skinBusy.value = true;
  try {
    await api.qookixClearSkin();
    skinImage.value = "";
    skinStatus.value = "approved";
    skinReason.value = "";
    message.success(t("qookix.skin.remove"));
  } catch (e) {
    message.error(String(e));
  } finally {
    skinBusy.value = false;
  }
}

// ---- 配置云同步（Phase 3）----
// 只同步白名单字段（内存/下载/外观/行为）+ 收藏；密钥与本机路径由 Rust 侧白名单挡掉。
const favorites = useFavoritesStore();
const syncBusy = ref(false);

async function syncUpload() {
  if (syncBusy.value) return;
  syncBusy.value = true;
  try {
    const r = await api.qookixSyncUpload();
    // 收藏是前端 localStorage 持有的，单独走通用 KV
    await api.qookixKvPut("favorites", favorites.snapshot());
    message.success(t("qookix.sync.uploadDone", { n: r.keys }));
  } catch (e) {
    message.error(String(e));
  } finally {
    syncBusy.value = false;
  }
}

function syncDownload() {
  if (syncBusy.value) return;
  dialog.warning({
    title: t("qookix.sync.download"),
    content: t("qookix.sync.downloadConfirm"),
    positiveText: t("common.confirm"),
    negativeText: t("common.cancel"),
    onPositiveClick: async () => {
      syncBusy.value = true;
      try {
        const r = await api.qookixSyncDownload();
        if (r.empty) {
          message.warning(t("qookix.sync.empty"));
          return;
        }
        // 重新拉一次设置，界面立即反映云端值
        await settings.load();
        let added = 0;
        try {
          const fav = await api.qookixKvGet("favorites");
          added = favorites.mergeFromCloud(fav.value);
        } catch (error) {
          message.error(String(error));
        }
        message.success(t("qookix.sync.downloadDone", { n: r.applied, fav: added }));
      } catch (e) {
        message.error(String(e));
      } finally {
        syncBusy.value = false;
      }
    },
  });
}

async function logoutQookix() {
  try {
    await qookixStore.logout();
    message.success(t("qookix.loggedOut"));
  } catch (e) {
    message.error(String(e));
  }
}

/** 刷新：三个第三方账号（QookiX / 拾木 / GitHub）的登录态一起重读 */
async function refreshAccounts() {
  shimuRefreshing.value = true;
  try {
    await shimuStore.init();
    if (shimuStore.loggedIn) {
      try {
        await api.shimuUserInfo();
        await shimuStore.refreshStatus();
      } catch {
        /* 资料刷新失败不影响登录态，保持原有显示 */
      }
    }
    // 另外两个账号各自独立，失败互不影响
    await Promise.all([qookixStore.refreshStatus(), loadCloudAccount()]);
    message.success(t("settings.accounts.refreshDone"));
  } catch (e) {
    message.error(shimuErrorMessage(e));
  } finally {
    shimuRefreshing.value = false;
  }
}

function openShimuSite() {
  const host = shimuStore.config?.host ?? "https://www.mcimg.com";
  openUrl(host).catch((e: unknown) => message.error(String(e)));
}

function logoutShimu() {
  dialog.warning({
    title: t("settings.accounts.logoutTitle"),
    content: t("settings.accounts.logoutContent"),
    positiveText: t("common.ok"),
    negativeText: t("common.cancel"),
    onPositiveClick: async () => {
      try {
        await shimuStore.logout();
        message.success(t("settings.accounts.logoutDone"));
      } catch (e) {
        message.error(shimuErrorMessage(e));
      }
    },
  });
}

async function loadCloudAccount() {
  try {
    const s = await api.cloudSyncStatus();
    cloudAccount.value = s.connected ? s.account : "";
    cloudRepo.value = s.repoName ?? "";
  } catch {
    cloudAccount.value = "";
  }
}
onMounted(loadCloudAccount);

async function logoutGithub() {
  dialog.warning({
    title: t("settings.storage.logoutGithub"),
    content: t("settings.storage.logoutConfirm"),
    positiveText: t("common.ok"),
    negativeText: t("common.cancel"),
    onPositiveClick: async () => {
      try {
        await api.cloudSyncDisconnect();
        message.success(t("cloudSync.disconnected"));
        await loadCloudAccount();
      } catch (e) {
        message.error(String(e));
      }
    },
  });
}

onActivated(() => {
  void shimuStore.init();
  void qookixStore.refreshStatus();
});
watch(() => props.cloudOpen, (visible) => {
  if (!visible) void loadCloudAccount();
});
</script>

<template>
  <div v-if="settings.settings" class="settings-body">
    <div class="settings-pane">
      <div class="grid storage-grid">
        <div class="card glass storage-card" id="accounts-thirdparty">
          <div class="storage-header">
            <h3>{{ t("settings.accounts.thirdParty") }}</h3>
            <div class="storage-actions">
              <button class="mini-btn" :disabled="shimuRefreshing" @click="refreshAccounts">
                <IconRefresh class="btn-icon" />
                {{ shimuRefreshing ? t("settings.accounts.refreshing") : t("settings.accounts.refresh") }}
              </button>
            </div>
          </div>

          <div class="acct-list">
            <!-- QookiX 账号（可选）：好友与联机房间目录 -->
            <div class="acct-row">
              <div class="acct-avatar acct-avatar-ph qk-avatar">
                <img src="/app-icon.png" alt="" draggable="false" />
              </div>
              <div class="acct-info">
                <div class="acct-name">
                  <span>{{ t("qookix.title") }}</span>
                  <span class="acct-badge" :class="qookixBadge">{{ qookixBadgeLabel }}</span>
                  <span v-if="qookixMcVerified" class="acct-badge on">
                    {{ t("qookix.mcVerified") }}
                  </span>
                  <span v-else-if="qookixMcName" class="acct-badge">
                    {{ t("qookix.mcUnverified") }}
                  </span>
                  <span v-if="qookixNickStatus === 'pending'" class="acct-badge">
                    {{ t("qookix.nicknamePending") }}
                  </span>
                  <span v-else-if="qookixNickStatus === 'rejected'" class="acct-badge">
                    {{ t("qookix.nicknameRejected") }}
                  </span>
                </div>
                <div class="acct-meta">{{ qookixMetaText }}</div>
                <div v-if="qookixNickStatus !== 'approved'" class="hint">
                  {{ t("qookix.nicknameReviewHint") }}
                  <template v-if="qookixNickReason">（{{ qookixNickReason }}）</template>
                </div>
              </div>
              <div class="acct-actions">
                <template v-if="qookixStore.loggedIn">
                  <button v-if="canBindMc" class="mini-btn" @click="bindCurrentMc">
                    {{ t("qookix.bindMc") }}
                  </button>
                  <template v-else-if="qookixMcVerified">
                    <button class="mini-btn" @click="bindCurrentMc">
                      {{ t("qookix.refreshMcName") }}
                    </button>
                    <button class="mini-btn danger" @click="unbindMc">
                      {{ t("qookix.unbindMc") }}
                    </button>
                  </template>
                  <button class="mini-btn" @click="openEmailChange">
                    {{ t("qookix.changeEmail") }}
                  </button>
                  <button class="mini-btn" @click="openChangePassword">
                    {{ t("qookix.password.title") }}
                  </button>
                  <button class="mini-btn" @click="openTwoFactor">
                    {{ tfaEnabled ? t("qookix.twofa.statusOn") : t("qookix.twofa.open") }}
                  </button>
                  <button class="mini-btn danger" @click="logoutQookix">
                    {{ t("qookix.logout") }}
                  </button>
                </template>
                <button v-else class="mini-btn primary" @click="openQookixLogin">
                  {{ t("qookix.login") }}
                </button>
              </div>
            </div>

            <!-- 隐私：向好友显示「正在玩什么」（读的是本机运行中的实例） -->
            <div v-if="qookixStore.loggedIn" class="switch-row acct-switch">
              <span>{{ t("qookix.shareActivity") }}</span>
              <button
                class="toggle"
                :class="{ on: settings.settings.share_activity !== false }"
                role="switch"
                :aria-checked="settings.settings.share_activity !== false"
                @click="settings.patch({ share_activity: settings.settings.share_activity === false })"
              >
                <span class="knob"></span>
              </button>
            </div>
            <p v-if="qookixStore.loggedIn" class="hint acct-switch-hint">
              {{ t("qookix.shareActivityHint") }}
            </p>

            <!-- 我的皮肤：QookiX 账号自带托管（替代第三方皮肤站） -->
            <div v-if="qookixStore.loggedIn" class="skin-box">
              <div class="skin-preview">
                <canvas ref="skinCanvas" width="88" height="150"></canvas>
              </div>
              <div class="skin-info">
                <div class="skin-title">{{ t("qookix.skin.title") }}</div>
                <p class="skin-hint">{{ t("qookix.skin.hint") }}</p>
                <p v-if="skinStatus !== 'approved'" class="skin-hint">
                  {{ skinStatus === "rejected" ? t("qookix.skinRejected") : t("qookix.skinPending") }}
                  · {{ t("qookix.skinReviewHint") }}
                  <template v-if="skinReason">（{{ skinReason }}）</template>
                </p>
                <div class="skin-ops">
                  <button class="mini-btn primary" :disabled="skinBusy" @click="pickSkin">
                    {{ skinBusy ? t("qookix.skin.uploading") : t("qookix.skin.upload") }}
                  </button>
                  <button
                    v-if="skinImage"
                    class="mini-btn danger"
                    :disabled="skinBusy"
                    @click="clearSkin"
                  >
                    {{ t("qookix.skin.remove") }}
                  </button>
                  <n-select
                    v-model:value="skinModel"
                    size="small"
                    :options="skinModelOptions"
                    style="width: 104px"
                  />
                </div>
                <input
                  ref="skinFileRef"
                  type="file"
                  accept="image/png"
                  class="skin-file"
                  @change="onSkinFile"
                />
              </div>
            </div>

            <!-- 配置云同步：换台电脑登录就能恢复偏好与收藏 -->
            <div v-if="qookixStore.loggedIn" class="sync-box">
              <div class="sync-head">
                <span class="sync-title">{{ t("qookix.sync.title") }}</span>
                <div class="sync-ops">
                  <button class="mini-btn" :disabled="syncBusy" @click="syncUpload">
                    {{ syncBusy ? t("qookix.sync.busy") : t("qookix.sync.upload") }}
                  </button>
                  <button class="mini-btn" :disabled="syncBusy" @click="syncDownload">
                    {{ t("qookix.sync.download") }}
                  </button>
                </div>
              </div>
              <p class="hint sync-hint">{{ t("qookix.sync.hint") }}</p>
            </div>

            <!-- 拾木空间：投影下载 / 付费内容 -->
            <div class="acct-row">
              <img v-if="shimuAvatar" class="acct-avatar" :src="shimuAvatar" alt="" />
              <div v-else class="acct-avatar acct-avatar-ph">
                {{ shimuStore.loggedIn ? (shimuStore.nickname.slice(0, 1) || "拾") : "拾" }}
              </div>
              <div class="acct-info">
                <div class="acct-name">
                  <span>{{ t("shimu.title") }}</span>
                  <span class="acct-badge" :class="shimuBadge">{{ shimuBadgeLabel }}</span>
                </div>
                <div class="acct-meta">{{ shimuMetaText }}</div>
              </div>
              <div class="acct-actions">
                <button class="mini-btn" @click="openShimuSite">
                  <IconExternal class="btn-icon" />{{ t("shimu.openSite") }}
                </button>
                <template v-if="shimuStore.loggedIn">
                  <button class="mini-btn" @click="openShimuLogin">{{ t("shimu.reauth") }}</button>
                  <button class="mini-btn danger" @click="logoutShimu">{{ t("shimu.logout") }}</button>
                </template>
                <button v-else class="mini-btn primary" @click="openShimuLogin">
                  {{ t("shimu.login") }}
                </button>
              </div>
            </div>
            <p v-if="shimuExpired" class="acct-warn inline">
              {{ t("settings.accounts.expiredHint") }}
            </p>

            <!-- GitHub：云存档与时光机共用（原在「存储」页，现统一放账号页） -->
            <div class="acct-row">
              <div class="acct-avatar acct-avatar-ph gh-avatar">
                <IconGithub />
              </div>
              <div class="acct-info">
                <div class="acct-name">
                  <span>{{ t("settings.storage.githubAccount") }}</span>
                  <span class="acct-badge" :class="{ on: !!cloudAccount }">
                    {{ cloudAccount ? t("settings.accounts.statusOn") : t("settings.storage.notConnected") }}
                  </span>
                </div>
                <div class="acct-meta">
                  <template v-if="cloudAccount">
                    <button
                      class="gh-user"
                      :title="t('settings.about.githubHome')"
                      @click="openUrl(`https://github.com/${cloudAccount}`)"
                    >
                      {{ cloudAccount }}
                    </button>
                    <span v-if="cloudRepo"> · {{ cloudRepo }}</span>
                  </template>
                  <template v-else>{{ t("settings.storage.cloudHint") }}</template>
                </div>
              </div>
              <div class="acct-actions">
                <button v-if="cloudAccount" class="mini-btn danger" @click="logoutGithub">
                  {{ t("settings.storage.logoutGithub") }}
                </button>
                <button v-else class="mini-btn primary" @click="emit('openCloud')">
                  {{ t("settings.storage.loginGithub") }}
                </button>
              </div>
            </div>

            <!-- 内网穿透（frp）：第三方面板，用于把本机端口映射到公网。
                 默认隐藏，只有开发者模式里打开「显示内网穿透」后才出现 -->
            <div v-if="showFrpFeatures" class="acct-row">
              <div class="acct-avatar acct-avatar-ph">
                <IconGlobe v-if="false" />
                {{ "穿" }}
              </div>
              <div class="acct-info">
                <div class="acct-name">
                  <span>{{ t("frp.title") }}</span>
                </div>
                <div class="acct-meta">{{ t("frp.loginHint") }}</div>
              </div>
              <div class="acct-actions">
                <button class="mini-btn primary" @click="openFrp">{{ t("frp.manage") }}</button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- 第三方账号授权弹窗（拾木空间 OAuth）：账号页与发现页投影共用同一套流程 -->
    <ShimuLoginDialog ref="shimuLoginRef" />

    <!-- QookiX 账号（可选）：好友 / 联机房间目录 -->
    <QookixLoginDialog ref="qookixLoginRef" />

    <!-- 内网穿透（frp）：登录面板 → 启停隧道；功能隐藏时连弹窗都不挂载 -->
    <FrpDialog v-if="showFrpFeatures" ref="frpRef" />

    <!-- 修改邮箱：验证信发到新邮箱，点开后才会生效 -->
    <n-modal class="settings-dialog"
      v-model:show="emailOpen"
      preset="card"
      :title="t('qookix.changeEmail')"
      style="width: 420px; max-width: 92vw"
      :bordered="false"
    >
      <div class="acct-modal">
        <p class="acct-modal-hint">{{ t("qookix.emailDialogHint") }}</p>
        <input
          v-model="emailDraft"
          class="ms-cid-input"
          :placeholder="t('qookix.newEmailPlaceholder')"
          spellcheck="false"
          @keyup.enter="submitEmailChange"
        />
        <div class="acct-modal-ops">
          <button class="mini-btn" @click="emailOpen = false">{{ t("common.cancel") }}</button>
          <button class="mini-btn primary" :disabled="emailBusy" @click="submitEmailChange">
            {{ t("qookix.sendMail") }}
          </button>
        </div>
      </div>
    </n-modal>

    <!-- 修改密码：成功后服务端换发新令牌，其他设备需要重新登录 -->
    <n-modal class="settings-dialog"
      v-model:show="pwdOpen"
      preset="card"
      :title="t('qookix.password.title')"
      style="width: 420px; max-width: 92vw"
      :bordered="false"
    >
      <div class="acct-modal">
        <label class="tfa-field">
          <span>{{ t("qookix.password.oldPlaceholder") }}</span>
          <n-input v-model:value="pwdOld" type="password" show-password-on="click" />
        </label>
        <label class="tfa-field">
          <span>{{ t("qookix.password.newPlaceholder") }}</span>
          <n-input v-model:value="pwdNew" type="password" show-password-on="click" />
        </label>
        <label class="tfa-field">
          <span>{{ t("qookix.password.confirmPlaceholder") }}</span>
          <n-input
            v-model:value="pwdNew2"
            type="password"
            show-password-on="click"
            @keyup.enter="submitChangePassword"
          />
        </label>
        <div class="acct-modal-ops">
          <button class="mini-btn" @click="pwdOpen = false">{{ t("common.cancel") }}</button>
          <button class="mini-btn primary" :disabled="pwdBusy" @click="submitChangePassword">
            {{ t("qookix.password.submit") }}
          </button>
        </div>
      </div>
    </n-modal>

    <!-- 两步验证：未开启时展示二维码 + 恢复码；已开启时可关闭 -->
    <n-modal class="settings-dialog"
      v-model:show="tfaOpen"
      preset="card"
      :title="tfaEnabled ? t('qookix.twofa.disableTitle') : t('qookix.twofa.setupTitle')"
      style="width: 460px; max-width: 92vw"
      :bordered="false"
    >
      <div class="acct-modal">
        <div v-if="tfaLoading" class="hint">…</div>
        <template v-else-if="!tfaEnabled">
          <p class="license-text">{{ t("qookix.twofa.setupIntro") }}</p>
          <div v-if="tfaSetup" class="tfa-setup">
            <img v-if="tfaQr" class="tfa-qr" :src="tfaQr" alt="2FA QR" />
            <div class="tfa-secret">
              <div class="tfa-label">{{ t("qookix.twofa.secretLabel") }}</div>
              <code class="tfa-secret-value">{{ tfaSetup.secret }}</code>
            </div>
          </div>
          <div v-if="tfaRecovery.length" class="tfa-recovery">
            <div class="tfa-label">{{ t("qookix.twofa.recoveryTitle") }}</div>
            <p class="license-text">{{ t("qookix.twofa.recoveryHint") }}</p>
            <div class="tfa-codes">
              <code v-for="c in tfaRecovery" :key="c">{{ c }}</code>
            </div>
            <button class="mini-btn" @click="copyRecoveryCodes">{{ t("qookix.twofa.copy") }}</button>
          </div>
          <label class="tfa-field">
            <span>{{ t("qookix.twofa.enableTitle") }}</span>
            <n-input
              v-model:value="tfaCode"
              maxlength="16"
              :placeholder="t('qookix.twofa.codePlaceholder')"
              @keyup.enter="submit2faEnable"
            />
          </label>
          <div class="acct-modal-ops">
            <button class="mini-btn" @click="tfaOpen = false">{{ t("qookix.twofa.close") }}</button>
            <button class="mini-btn primary" :disabled="tfaBusy" @click="submit2faEnable">
              {{ t("qookix.twofa.enable") }}
            </button>
          </div>
        </template>
        <template v-else>
          <p class="license-text">{{ t("qookix.twofa.disableIntro") }}</p>
          <p v-if="tfaStatus?.recovery_remaining !== undefined" class="license-text">
            {{ t("qookix.twofa.recoveryRemaining", { n: tfaStatus.recovery_remaining }) }}
          </p>
          <label class="tfa-field">
            <span>{{ t("qookix.twofa.passwordPlaceholder") }}</span>
            <n-input v-model:value="tfaPassword" type="password" show-password-on="click" />
          </label>
          <label class="tfa-field">
            <span>{{ t("qookix.twofa.codeLabel") }}</span>
            <n-input
              v-model:value="tfaCode"
              maxlength="16"
              :placeholder="t('qookix.twofa.codePlaceholder')"
              @keyup.enter="submit2faDisable"
            />
          </label>
          <div class="acct-modal-ops">
            <button class="mini-btn" @click="tfaOpen = false">{{ t("qookix.twofa.close") }}</button>
            <button class="mini-btn danger" :disabled="tfaBusy" @click="submit2faDisable">
              {{ t("qookix.twofa.disable") }}
            </button>
          </div>
        </template>
      </div>
    </n-modal>
  </div>
</template>
