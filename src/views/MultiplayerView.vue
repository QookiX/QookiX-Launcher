<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { fmtBytes } from "../utils/format";
import { CORE_COLORS, CORE_LABELS } from "../utils/cores";
import { isAprilFools } from "../utils/versions";
import { useRouter } from "vue-router";
import { NModal, NInput, NProgress, NSelect, NSwitch, useMessage } from "naive-ui";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../api";
import { useAccountsStore } from "../stores/accounts";
import { useServersStore } from "../stores/servers";
import { useInstancesStore } from "../stores/instances";
import type { ServerCore, TerracottaDownloadProgress, TerracottaInfo } from "../types";
import { useSlidingIndicator } from "../composables/useSlidingIndicator";
import { showFrpFeatures } from "../composables/useDevFlags";
import {
  IconBox,
  IconCheck,
  IconClose,
  IconCopy,
  IconDoorOpen,
  IconDownload,
  IconExternal,
  IconGlobe,
  IconPlay,
  IconPlus,
  IconRefresh,
  IconServer,
  IconStop,
  IconTrash,
  IconUser,
  IconUsers,
} from "../components/icons";
import McMotd from "../components/McMotd.vue";
import FriendsPanel from "../components/FriendsPanel.vue";
import RoomDirectoryPanel from "../components/RoomDirectoryPanel.vue";
import QookixLoginDialog from "../components/QookixLoginDialog.vue";
import FrpPanel from "../components/FrpPanel.vue";
import { qookixErrorMessage, useQookixStore } from "../stores/qookix";
import { useQookixPrefsStore } from "../stores/qookixPrefs";
import { PLAY_TYPES } from "../utils/qookix";

const router = useRouter();
const { t } = useI18n();
const accounts = useAccountsStore();
const servers = useServersStore();
const instances = useInstancesStore();
const message = useMessage();

// ---- QookiX 可选账号（好友 / 房间目录） ----
const qookix = useQookixStore();
const qookixPrefs = useQookixPrefsStore();
const qookixLoginRef = ref<InstanceType<typeof QookixLoginDialog> | null>(null);
const shareBusy = ref(false);

function openQookixLogin() {
  qookixLoginRef.value?.open();
}

/** 把当前陶瓦房间分享到目录（好友可见 / 公开到大厅）；未登录时先弹登录 */
async function shareRoom(
  visibility: "friends" | "public",
  playType?: string,
  version?: string,
  title?: string,
  pack?: string,
  modsCount?: number,
  packNet?: string,
) {
  const code = tcRoomCodeFinal.value;
  if (!code) {
    message.warning(t("qookix.rooms.noCode"));
    return;
  }
  if (!qookix.loggedIn) {
    openQookixLogin();
    return;
  }
  shareBusy.value = true;
  try {
    await qookix.shareRoom(code, visibility, title, playType, version, pack, modsCount, packNet);
    message.success(
      visibility === "public" ? t("qookix.rooms.sharedPublic") : t("qookix.rooms.sharedFriends"),
    );
  } catch (e) {
    message.error(qookixErrorMessage(e));
  } finally {
    shareBusy.value = false;
  }
}

// ---- 公开到大厅：标题 / 玩法 / 版本（服务端用来做筛选，本机记住上次填的） ----
const publishOpen = ref(false);
const publishTitle = ref("");
const publishPlayType = ref<string | null>(null);
const publishVersion = ref("");
// 环境清单（Phase 2a）：房主可以顺带把实例的模组清单分享出去
const publishSharePack = ref(false);
const publishInstanceId = ref<string | null>(null);
const publishVisibility = ref<"friends" | "public">("friends");
const playTypeOptions = computed(() =>
  PLAY_TYPES.map((p) => ({ label: t(p.labelKey), value: p.value })),
);
const packInstanceOptions = computed(() =>
  instances.instances
    .filter((i) => i.installed)
    .map((i) => ({ label: i.name, value: i.id })),
);

/** 公开到大厅是唯一面向陌生人的入口，要求正版已验证；好友分享不受影响 */
const canPublishLobby = computed(() => qookix.loggedIn && !!qookix.user?.mc_verified);

/** 统一的分享弹窗：好友分享不要求正版，公开到大厅才要求 */
function openShareDialog(visibility: "friends" | "public") {
  if (!tcRoomCodeFinal.value) {
    message.warning(t("qookix.rooms.noCode"));
    return;
  }
  if (!qookix.loggedIn) {
    openQookixLogin();
    return;
  }
  if (visibility === "public" && !canPublishLobby.value) {
    // 不打开弹窗，直接说清楚缺什么、去哪儿补
    message.warning(t("qookix.rooms.needVerified"));
    return;
  }
  publishVisibility.value = visibility;
  // 上次填过什么就带出来，别每次都重敲
  publishTitle.value = qookixPrefs.share.title;
  publishPlayType.value = qookixPrefs.share.playType || null;
  publishVersion.value = qookixPrefs.share.version;
  publishSharePack.value = false;
  publishInstanceId.value = null;
  if (!instances.instances.length) void instances.load();
  publishOpen.value = true;
}

function openPublishDialog() {
  openShareDialog("public");
}

/** 只分享给好友：也用上次的标题，邀请卡片里能认出来是什么房 */
function shareWithFriends() {
  openShareDialog("friends");
}

async function confirmPublish() {
  // 双保险：弹窗打开期间解绑了也不该发出去
  if (publishVisibility.value === "public" && !canPublishLobby.value) {
    publishOpen.value = false;
    message.warning(t("qookix.rooms.needVerified"));
    return;
  }
  const preset = {
    title: publishTitle.value.trim(),
    playType: publishPlayType.value ?? "",
    version: publishVersion.value.trim(),
  };
  qookixPrefs.saveSharePreset(preset);

  // 环境清单：清单 JSON 只几 KB，走服务端；文件本体永远走 P2P
  let pack: string | undefined;
  let modsCount: number | undefined;
  let packNet: string | undefined;
  if (publishSharePack.value) {
    if (!publishInstanceId.value) {
      message.warning(t("qookix.pack.instanceEmpty"));
      return;
    }
    try {
      // 起服务 = 生成清单 + 起 HTTP + 起 EasyTier 节点，一次拿全
      const s = await api.qookixPackServe(publishInstanceId.value);
      pack = s.manifest;
      modsCount = s.modsCount;
      packNet = s.packNet;
    } catch (e) {
      message.error(String(e));
      return;
    }
  } else {
    // 不共享环境就别留着上次的节点
    void api.qookixPackStopServe().catch(() => {});
  }

  publishOpen.value = false;
  await shareRoom(
    publishVisibility.value,
    preset.playType || undefined,
    preset.version || undefined,
    preset.title || undefined,
    pack,
    modsCount,
    packNet,
  );
}

async function stopSharing() {
  shareBusy.value = true;
  try {
    await qookix.stopShare();
    message.success(t("qookix.rooms.unshared"));
  } catch (e) {
    message.error(qookixErrorMessage(e));
  } finally {
    shareBusy.value = false;
  }
}

/** 从目录或邀请加入房间后，让陶瓦状态轮询立刻跑起来 */
function onQookixJoined() {
  void refreshTc();
  startTcPoll();
}

const tab = ref<"servers" | "rooms" | "friends" | "tunnel">("servers");
const tabSegRef = ref<HTMLElement | null>(null);
const TAB_INDEX: Record<"servers" | "rooms" | "friends" | "tunnel", number> = {
  servers: 0,
  rooms: 1,
  friends: 2,
  tunnel: 3,
};
const { indicatorStyle: tabSegStyle, refresh: refreshTabSeg } = useSlidingIndicator(
  tabSegRef,
  () => Array.from(tabSegRef.value?.querySelectorAll<HTMLElement>("button") ?? []),
  () => TAB_INDEX[tab.value],
);
watch(tab, () => nextTick(() => refreshTabSeg()));

// 内网穿透默认隐藏（开发者模式里的开关才放行）：功能被藏起来时
// 不能把用户留在已经消失的页签上，退回到「服务器」。
watch(
  showFrpFeatures,
  (on) => {
    if (!on && tab.value === "tunnel") tab.value = "servers";
  },
  { immediate: true },
);

// 内网穿透的两个子页签（内容在 FrpPanel，页签本身放在顶栏）
const frpSub = ref<"panel" | "local">("panel");
const frpSubRef = ref<HTMLElement | null>(null);
const FRP_SUB_INDEX: Record<"panel" | "local", number> = { panel: 0, local: 1 };
const { indicatorStyle: frpSubStyle, refresh: refreshFrpSub } = useSlidingIndicator(
  frpSubRef,
  () => Array.from(frpSubRef.value?.querySelectorAll<HTMLElement>("button") ?? []),
  () => FRP_SUB_INDEX[frpSub.value],
);
const frpPanelCount = ref(0);
const frpLocalCount = ref(0);
function onFrpCounts(panel: number, local: number) {
  frpPanelCount.value = panel;
  frpLocalCount.value = local;
}
// 切页签、切子页签、计数徽标出现/变化都要重新测量：
// 徽标是异步来的，第一次挂载时按钮还窄着，量早了指示器就会短一截。
watch([frpSub, tab, frpPanelCount, frpLocalCount], () => nextTick(() => refreshFrpSub()));

// ---- 版本清单 ----
const versions = ref<{ id: string; type: string; releaseTime: string }[]>([]);
const versionCat = ref<string>("release");

const filteredVersions = computed(() =>
  versions.value.filter((v) => {
    if (versionCat.value === "release") return v.type === "release" || v.type.startsWith("old_");
    if (versionCat.value === "april") return isAprilFools(v);
    return v.type === "snapshot" && !isAprilFools(v);
  }),
);

// ---- 服务器核心选项 ----
const CORES: { value: ServerCore; label: string; desc: string }[] = [
  { value: "vanilla", label: "Vanilla", desc: "multiplayer.core.vanillaDesc" },
  { value: "paper", label: "Paper", desc: "multiplayer.core.paperDesc" },
  { value: "spigot", label: "Spigot", desc: "multiplayer.core.spigotDesc" },
  { value: "purpur", label: "Purpur", desc: "multiplayer.core.purpurDesc" },
  { value: "forge", label: "Forge", desc: "multiplayer.core.forgeDesc" },
  { value: "fabric", label: "Fabric", desc: "multiplayer.core.fabricDesc" },
];

// ---- 创建对话框（仅名称 / 核心 / 版本）----
type ServerDialog = {
  name: string;
  core: ServerCore;
  mc_version: string;
};

const dialog = ref<ServerDialog | null>(null);
const saving = ref(false);

// ---- 核心下载进度 ----
const installing = ref<{ name: string; phase: string; done: number; total: number } | null>(null);

const installPct = computed(() => {
  const ins = installing.value;
  if (!ins || ins.total <= 0) return 0;
  return Math.min(100, Math.round((ins.done / ins.total) * 100));
});
const installHasBytes = computed(() => {
  const ins = installing.value;
  return !!ins && ins.total > 1;
});
function openCreate() {
  versionCat.value = "release";
  dialog.value = { name: "", core: "paper", mc_version: "" };
}

const canSave = computed(() => {
  const d = dialog.value;
  return !!d && !!d.core && !!d.mc_version;
});

async function saveDialog() {
  const d = dialog.value;
  if (!d) return;
  if (!d.mc_version) return message.warning(t("multiplayer.msg.selectVersion"));
  saving.value = true;
  try {
    const s = await servers.create(d.name, d.core, d.mc_version);
    dialog.value = null;
    saving.value = false;

    installing.value = { name: s.name, phase: t("multiplayer.msg.preparingCore"), done: 0, total: 0 };
    const un = await listen<{ serverId: string; phase: string; done: number; total: number }>(
      "server://install-progress",
      (ev) => {
        if (installing.value) {
          installing.value = {
            ...installing.value,
            phase: ev.payload.phase,
            done: ev.payload.done ?? 0,
            total: ev.payload.total ?? 0,
          };
        }
      },
    );
    try {
      await servers.installCore(s.id);
      message.success(t("multiplayer.msg.coreReady", { name: s.name }));
    } catch (e) {
      message.error(String(e));
    }
    un();
    installing.value = null;
    router.push(`/multiplayer/${s.id}`);
  } catch (e) {
    message.error(String(e));
    saving.value = false;
    installing.value = null;
  }
}

// ---- 删除确认 ----
const confirmState = ref<{ id: string; name: string } | null>(null);
function confirmDelete(id: string, name: string) {
  confirmState.value = { id, name };
}
async function doDelete() {
  const c = confirmState.value;
  if (!c) return;
  try {
    await servers.remove(c.id);
    message.success(t("multiplayer.msg.deleted", { name: c.name }));
  } catch (e) {
    message.error(String(e));
  }
  confirmState.value = null;
}

// ---- 启动 / 停止 ----
async function toggleRun(id: string, running: boolean) {
  try {
    if (running) {
      await servers.stop(id);
      message.success(t("multiplayer.msg.stopped"));
    } else {
      await servers.start(id);
      message.success(t("multiplayer.msg.started"));
    }
  } catch (e) {
    message.error(String(e));
  }
}

// 服务器卡片逐个进场（同首页 pin-card / 内容页卡片的做法）：延迟由内联 style 下发给每张卡片。
// 初次加载、切标签页回来时整批挂载播放级联；新建单个服务器时只有新卡片进场。
// 数量多时自动收紧步长，保证整批级联在 ~0.7s 内跑完。
function cardDelay(i: number): string {
  const step = Math.min(0.045, 0.6 / Math.max(1, servers.servers.length));
  return `${(0.06 + i * step).toFixed(3)}s`;
}

// ---- 陶瓦联机（联机房间）----
const tc = ref<TerracottaInfo | null>(null);
const tcLoading = ref(false);
const tcBusy = ref(false);
const tcRoomCode = ref("");
// 创建房间的玩家名，默认读取当前账号名
const tcPlayerName = ref("");
const tcRoom = ref<Record<string, unknown> | null>(null);
const tcJoined = ref(false);
const tcDownloading = ref(false);
const tcDownloadProgress = ref<TerracottaDownloadProgress | null>(null);
let tcDlUnlisten: (() => void) | null = null;
let tcPollTimer: number | null = null;

const tcStateName = computed<string>(() => String(tcRoom.value?.state ?? "waiting"));
const tcRoomCodeFinal = computed<string>(() => {
  const s = tcRoom.value;
  if (!s) return "";
  const room = s.room;
  if (room && typeof room === "object") {
    const code = (room as Record<string, unknown>).code;
    if (code) return String(code);
  }
  return String(s.room ?? "");
});
const tcUrl = computed<string>(() => String(tcRoom.value?.url ?? ""));

// 解析玩家列表（profiles 数组），自适应常见字段名
const tcPlayers = computed<Record<string, unknown>[]>(() => {
  const raw = tcRoom.value?.profiles;
  if (!Array.isArray(raw)) return [];
  return raw.filter((p): p is Record<string, unknown> => !!p && typeof p === "object");
});
const playerName = (p: Record<string, unknown>): string =>
  String(p.name ?? p.username ?? p.playerName ?? p.displayName ?? t("multiplayer.msg.unknownPlayer"));
const playerUuid = (p: Record<string, unknown>): string =>
  String(p.uuid ?? p.id ?? "");
const playerPing = (p: Record<string, unknown>): string => {
  const v = p.ping ?? p.latency;
  return v === undefined || v === null ? "" : String(v);
};
const playerDesc = (p: Record<string, unknown>): string =>
  String(p.description ?? p.desc ?? p.comment ?? "");

const tcStateText = computed(() => {
  const map: Record<string, string> = {
    waiting: "multiplayer.tc.state.waiting",
    "host-scanning": "multiplayer.tc.state.hostScanning",
    "host-starting": "multiplayer.tc.state.hostStarting",
    "host-ok": "multiplayer.tc.state.hostOk",
    "guest-connecting": "multiplayer.tc.state.guestConnecting",
    "guest-starting": "multiplayer.tc.state.guestStarting",
    "guest-ok": "multiplayer.tc.state.guestOk",
    exception: "multiplayer.tc.state.exception",
  };
  return t(map[tcStateName.value] ?? "multiplayer.tc.state.syncing");
});

async function refreshTc() {
  tcLoading.value = true;
  try {
    tc.value = await api.terracottaDetect();
    if (tc.value?.running) startTcPoll();
  } catch (e) {
    message.error(String(e));
  } finally {
    tcLoading.value = false;
  }
}

const tcDownloadPercent = computed(() => {
  const p = tcDownloadProgress.value;
  if (!p) return 0;
  if (p.done || p.extracting) return 100;
  if (p.percent >= 0) return Math.min(100, p.percent);
  return 0;
});

const tcDownloadText = computed(() => {
  const p = tcDownloadProgress.value;
  if (!p) return t("multiplayer.tc.dlText.preparing");
  if (p.extracting) return t("multiplayer.tc.dlText.extracting");
  if (p.done) return t("multiplayer.tc.dlText.done");
  if (p.total > 0) {
    const mb = (v: number) => (v / 1024 / 1024).toFixed(1);
    return `${tcDownloadPercent.value}%（${mb(p.downloaded)} / ${mb(p.total)} MB）`;
  }
  return t("multiplayer.tc.dlText.fetching");
});

async function downloadTc() {
  if (tcDownloading.value || tcBusy.value) return;
  tcDownloading.value = true;
  tcDownloadProgress.value = { downloaded: 0, total: 0, percent: 0 };
  try {
    tcDlUnlisten = await listen<TerracottaDownloadProgress>(
      "terracotta://download-progress",
      (ev) => {
        tcDownloadProgress.value = ev.payload;
      },
    );
    await api.terracottaDownload();
    tcDownloadProgress.value = { downloaded: 1, total: 1, percent: 100, done: true };
    message.success(t("multiplayer.msg.tcInstalled"));
    await refreshTc();
  } catch (e) {
    message.error(String(e));
  } finally {
    if (tcDlUnlisten) {
      tcDlUnlisten();
      tcDlUnlisten = null;
    }
    tcDownloading.value = false;
  }
}

async function launchTc() {
  if (tcBusy.value) return;
  tcBusy.value = true;
  try {
    const l = await api.terracottaLaunch();
    if (tc.value) tc.value = { ...tc.value, running: true, port: l.port };
    startTcPoll();
    message.success(t("multiplayer.msg.tcStarted"));
  } catch (e) {
    message.error(String(e));
  } finally {
    tcBusy.value = false;
  }
}

async function stopTc() {
  try {
    await api.terracottaStop();
    stopTcPoll();
    if (tc.value) tc.value = { ...tc.value, running: false, port: null };
    tcRoom.value = null;
    message.success(t("multiplayer.msg.tcStopped"));
  } catch (e) {
    message.error(String(e));
  }
}

function openTcUi() {
  const p = tc.value?.port;
  if (p) openUrl(`http://127.0.0.1:${p}/`).catch(() => {});
}

// 正在进行中的状态：轮询时不会被空闲(waiting/空)状态覆盖
const TC_PENDING_STATES = new Set([
  "host-scanning",
  "host-starting",
  "guest-connecting",
  "guest-starting",
]);

async function pollTcState() {
  try {
    const s = (await api.terracottaStatus()) as Record<string, unknown> | null;
    const next = String(s?.state ?? "waiting");
    // 本地正处于进行中状态，而陶瓦返回空闲/空，说明还没进入对应状态，
    // 保留本地状态，避免界面掉回「创建/加入」面板
    if (TC_PENDING_STATES.has(tcStateName.value) && next === "waiting") {
      return;
    }
    tcRoom.value = s;
  } catch {
    // 轮询失败忽略，等待下一次
  }
}

function startTcPoll() {
  stopTcPoll();
  pollTcState();
  tcPollTimer = window.setInterval(pollTcState, 1500);
}

function stopTcPoll() {
  if (tcPollTimer !== null) {
    window.clearInterval(tcPollTimer);
    tcPollTimer = null;
  }
}

async function createRoom() {
  if (tcBusy.value) return;
  tcBusy.value = true;
  // 点击后立即进入扫描引导，避免等待轮询期间掉回创建/加入面板
  tcRoom.value = { state: "host-scanning" };
  try {
    const name = tcPlayerName.value.trim();
    await api.terracottaCreateRoom(name || undefined);
    await pollTcState();
  } catch (e) {
    tcRoom.value = null;
    message.error(String(e));
  } finally {
    tcBusy.value = false;
  }
}

async function joinRoom() {
  if (tcBusy.value) return;
  const code = tcRoomCode.value.trim();
  if (!code) return message.warning(t("multiplayer.msg.enterRoomCode"));
  tcBusy.value = true;
  // 点击后立即进入连接引导，避免等待轮询期间掉回创建/加入面板
  tcRoom.value = { state: "guest-connecting" };
  try {
    const name = tcPlayerName.value.trim();
    await api.terracottaJoinRoom(code, name || undefined);
    await pollTcState();
  } catch (e) {
    tcRoom.value = null;
    message.error(String(e));
  } finally {
    tcBusy.value = false;
  }
}

async function leaveRoom() {
  try {
    await api.terracottaLeave();
    tcRoom.value = null;
    message.success(t("multiplayer.msg.leftRoom"));
  } catch (e) {
    message.error(String(e));
  }
}

async function copyRoomCode() {
  if (!tcRoomCodeFinal.value) return;
  try {
    await navigator.clipboard.writeText(tcRoomCodeFinal.value);
    tcJoined.value = true;
    setTimeout(() => (tcJoined.value = false), 1600);
  } catch {
    // 忽略剪贴板失败
  }
}

// 默认读取当前账号名，切换账号时若玩家名未被手动修改则同步更新
watch(
  () => accounts.current?.username ?? "",
  (name) => {
    if (name && (!tcPlayerName.value || tcPlayerName.value === "")) tcPlayerName.value = name;
  },
  { immediate: true },
);

// —— 本机局域网 IP ——
// 服务器卡片要显示「别人该连哪个地址」，所以取本机首选局域网 IPv4
// （后端复用局域网同步那套网卡打分：真实网卡优先、虚拟网卡降权、
// Clash 之类的假 IP 直接拒绝）。取不到（离线 / 无网卡）就回退 localhost——
// 服务器绑的是 0.0.0.0，两个地址在同一个界面上都可用。
const lanIp = ref("");
let lanIpAt = 0;
async function refreshLanIp(force = false) {
  const now = Date.now();
  // 切窗口会频繁触发 focus，3 秒内不重复枚举网卡
  if (!force && now - lanIpAt < 3000) return;
  lanIpAt = now;
  try {
    lanIp.value = (await api.lanLocalIp()) ?? "";
  } catch {
    lanIp.value = "";
  }
}
function serverAddress(port: number) {
  return `${lanIp.value || "localhost"}:${port}`;
}

onMounted(async () => {
  servers.load();
  refreshTc();
  void refreshLanIp(true);
  window.addEventListener("focus", onWinFocus);
  try {
    const m = await api.getVersionManifest();
    versions.value = m.versions;
  } catch (e) {
    message.error(String(e));
  }
});

// 换 Wi-Fi / 插拔网线后地址会变，回到窗口时顺手刷一次
function onWinFocus() {
  void refreshLanIp();
}
onUnmounted(() => window.removeEventListener("focus", onWinFocus));

// 标题栏的“创建服务器”按钮仅在“服务器”标签页显示
watch(tab, (v) => servers.setCanCreate(v === "servers"), { immediate: true });
onUnmounted(() => servers.setCanCreate(false));

// 响应标题栏的“创建服务器”按钮
watch(
  () => servers.createRequest,
  (n) => {
    if (n > 0) openCreate();
  },
);

onUnmounted(() => stopTcPoll());
</script>

<template>
  <div id="mp-root" class="mp-view" data-tauri-drag-region>
    <div class="mp-head">
      <div id="mp-tabs" ref="tabSegRef" class="qx-seg mode-tabs">
        <div class="qx-seg__indicator" :style="tabSegStyle"></div>
        <button :class="{ active: tab === 'servers' }" @click="tab = 'servers'">
          <IconServer /> {{ t('multiplayer.tab.servers') }}
        </button>
        <button :class="{ active: tab === 'rooms' }" @click="tab = 'rooms'">
          <IconUsers /> {{ t('multiplayer.tab.rooms') }}
        </button>
        <button :class="{ active: tab === 'friends' }" @click="tab = 'friends'">
          <IconUser /> {{ t('qookix.tabs.friends') }}
          <span v-if="qookix.pendingCount" class="tab-badge">{{ qookix.pendingCount }}</span>
        </button>
        <button v-if="showFrpFeatures" :class="{ active: tab === 'tunnel' }" @click="tab = 'tunnel'">
          <IconGlobe /> {{ t("frp.title") }}
        </button>
      </div>

      <!-- 内网穿透的子页签：和主标签同一行，靠右。
           功能开启时常驻挂载、只切显隐（v-if 每次进页签都重新挂载，
           一挂载就要重新测量，滑块会有一次可见的定位跳动）；
           整个功能默认隐藏，靠开发者页的开关整体放行。 -->
      <div
        v-if="showFrpFeatures"
        ref="frpSubRef"
        class="qx-seg mp-subtabs"
        :class="{ hidden: tab !== 'tunnel' }"
      >
        <div class="qx-seg__indicator" :style="frpSubStyle"></div>
        <button :class="{ active: frpSub === 'panel' }" @click="frpSub = 'panel'">
          {{ t("frp.panelTunnels") }}
          <!-- 数量为 0 时直接不渲染：以前用 visibility:hidden 占位会在按钮里
               留下一整块看不见的空白。数量到位时 watch 会重测滑块并平滑过渡。 -->
          <span v-if="frpPanelCount" class="qx-badge mp-sub-count">{{ frpPanelCount }}</span>
        </button>
        <button :class="{ active: frpSub === 'local' }" @click="frpSub = 'local'">
          {{ t("frp.localTunnels") }}
          <span v-if="frpLocalCount" class="qx-badge mp-sub-count">{{ frpLocalCount }}</span>
        </button>
      </div>
    </div>

    <!-- 服务器 -->
    <template v-if="tab === 'servers'">
      <div v-if="servers.count" class="grid">
        <div
          v-for="(s, i) in servers.servers"
          :key="s.id"
          class="server-card glass clickable"
          :style="{ animationDelay: cardDelay(i) }"
          @click="router.push(`/multiplayer/${s.id}`)"
        >
          <div class="card-top">
            <div class="icon"><IconServer /></div>
            <div class="title-wrap">
              <div class="name text-ellipsis">{{ s.name }}</div>
              <div class="meta">
                <span
                  class="core-badge"
                  :style="{ color: CORE_COLORS[s.core], borderColor: CORE_COLORS[s.core] }"
                >{{ CORE_LABELS[s.core] }}</span>
                <span class="mc">{{ s.mc_version }}</span>
              </div>
              <!-- 连接地址独占一行：优先本机局域网 IP（朋友能直接连），取不到回退 localhost。
                   和徽标/版本挤在一行时，窄卡片下会被挤压省略，单独成行后地址永远完整可见。 -->
              <div class="addr text-ellipsis">{{ serverAddress(s.port) }}</div>
            </div>
          </div>
          <div class="card-motd">
            <McMotd :text="s.motd" default-text="A Minecraft Server" />
          </div>
          <div class="card-foot" @click.stop>
            <div class="foot-info">
              <span class="status" :class="{ on: servers.isRunning(s.id) }">
                <i v-if="servers.isRunning(s.id)" class="status-dot"></i>
                {{ servers.isRunning(s.id) ? t('multiplayer.card.running') : t('multiplayer.card.stopped') }}
              </span>
            </div>
            <div class="actions">
              <button
                class="icon-btn play"
                :class="{ stop: servers.isRunning(s.id) }"
                :title="servers.isRunning(s.id) ? t('multiplayer.card.stop') : t('multiplayer.card.start')"
                @click="toggleRun(s.id, servers.isRunning(s.id))"
              >
                <IconStop v-if="servers.isRunning(s.id)" />
                <IconPlay v-else />
              </button>
              <button class="icon-btn danger" :title="t('multiplayer.card.delete')" @click="confirmDelete(s.id, s.name)">
                <IconTrash />
              </button>
            </div>
          </div>
        </div>
      </div>

      <div v-else class="empty glass">
        <div class="empty-icon"><IconServer /></div>
        <p>{{ t('multiplayer.empty.noServers') }}</p>
        <button class="qx-btn qx-btn--primary" @click="openCreate">{{ t('multiplayer.empty.createFirst') }}</button>
      </div>
    </template>

    <!-- 联机房间（陶瓦联机） -->
    <template v-else-if="tab === 'rooms'">
      <!-- 好友房间 / 邀请（可选账号） -->
      <RoomDirectoryPanel @login="openQookixLogin" @joined="onQookixJoined" />

      <!-- 检测中 -->
      <div v-if="tcLoading || !tc" class="empty glass">
        <div class="empty-icon"><IconRefresh /></div>
        <p>{{ t('multiplayer.tc.detecting') }}</p>
      </div>

      <!-- 未安装 -->
      <div v-else-if="!tc.found" class="tc-download">
        <div class="tc-hero glass">
          <div v-if="tc.icon" class="tc-hero-icon tc-icon-img">
            <img :src="tc.icon" :alt="t('multiplayer.tc.name')" />
          </div>
          <div class="tc-hero-text">
            <h2>{{ t('multiplayer.tc.name') }}</h2>
            <p>{{ t('multiplayer.tc.intro') }}</p>
          </div>
        </div>

        <div class="glass tc-dl-card">
          <div class="tc-dl-title"><IconDownload /> {{ t('multiplayer.tc.needDownload') }}</div>
          <p class="tc-dl-desc">
            {{ t('multiplayer.tc.downloadDesc') }}
          </p>

          <div v-if="tcDownloading" class="tc-dl-progress">
            <div class="tc-dl-bar">
              <div class="tc-dl-fill" :style="{ width: tcDownloadPercent + '%' }"></div>
            </div>
            <span class="tc-dl-text">{{ tcDownloadText }}</span>
          </div>
          <div v-else class="tc-dl-actions">
            <button class="qx-btn qx-btn--primary" :disabled="tcLoading" @click="downloadTc">
              <IconDownload /> {{ t('multiplayer.tc.download') }}
            </button>
            <button class="qx-btn" :disabled="tcLoading" @click="refreshTc">
              <IconRefresh /> {{ t('multiplayer.tc.recheck') }}
            </button>
          </div>

          <p class="tc-hint">
            {{ t('multiplayer.tc.installHintPrefix') }} <code>%LOCALAPPDATA%\Programs\Terracotta\Terracotta.exe</code>
          </p>
        </div>
      </div>

      <!-- 已安装未运行 -->
      <div v-else-if="!tc.running" class="tc-download">
        <div class="tc-hero glass">
          <div v-if="tc.icon" class="tc-hero-icon tc-icon-img">
            <img :src="tc.icon" :alt="t('multiplayer.tc.name')" />
          </div>
          <div class="tc-hero-text">
            <h2>{{ t('multiplayer.tc.name') }}</h2>
            <p>{{ t('multiplayer.tc.installed') }}</p>
          </div>
          <div class="tc-hero-ops">
            <button class="tc-hero-launch" :disabled="tcBusy" @click="launchTc">
              <IconPlay class="tc-hero-launch-icon" />
              <span>{{ t('multiplayer.tc.launch') }}</span>
            </button>
          </div>
        </div>
      </div>

      <!-- 运行中 -->
      <div v-else class="tc-download">
        <div class="tc-hero glass">
          <div v-if="tc.icon" class="tc-hero-icon tc-icon-img">
            <img :src="tc.icon" :alt="t('multiplayer.tc.name')" />
          </div>
          <div class="tc-hero-text">
            <h2>{{ t('multiplayer.tc.name') }}</h2>
            <p class="tc-state">{{ tcStateText }}</p>
          </div>
          <div class="tc-hero-ops">
            <button class="op" @click="openTcUi"><IconExternal /> {{ t('multiplayer.tc.openUi') }}</button>
            <button class="op stop" @click="stopTc"><IconStop /> {{ t('multiplayer.tc.stop') }}</button>
          </div>
        </div>

        <!-- 主机：房间已创建 -->
        <div v-if="tcStateName === 'host-ok'" class="glass tc-code-card">
          <div class="tc-code-label"><IconUsers /> {{ t('multiplayer.tc.roomCreated') }}</div>
          <div class="tc-code-value mono">{{ tcRoomCodeFinal }}</div>
          <p class="tc-code-hint">{{ t('multiplayer.tc.roomCodeHint') }}</p>
          <div class="tc-code-actions">
            <button class="qx-btn qx-btn--primary" @click="copyRoomCode">
              <IconCopy /> {{ tcJoined ? t('multiplayer.tc.copied') : t('multiplayer.tc.copyCode') }}
            </button>
            <button class="qx-btn" @click="leaveRoom"><IconClose /> {{ t('multiplayer.tc.leaveRoom') }}</button>
          </div>

          <!-- 分享到目录（可选账号） -->
          <div class="tc-share-row">
            <template v-if="qookix.share">
              <span class="tc-share-dot" :class="{ pub: qookix.share?.visibility === 'public' }"></span>
              <span class="tc-share-text">
                {{
                  qookix.share?.visibility === "public"
                    ? t("qookix.rooms.sharedPublic")
                    : t("qookix.rooms.sharedFriends")
                }}
              </span>
              <span class="tc-share-sep"></span>
              <button class="tc-share-link" :disabled="shareBusy" @click="stopSharing">
                {{ t("qookix.rooms.stopShare") }}
              </button>
            </template>
            <template v-else>
              <button class="tc-share-link" :disabled="shareBusy" @click="shareWithFriends">
                <IconUsers /> {{ t("qookix.rooms.shareFriends") }}
              </button>
              <span class="tc-share-sep"></span>
              <button
                class="tc-share-link"
                :class="{ locked: !canPublishLobby }"
                :disabled="shareBusy"
                :title="canPublishLobby ? '' : t('qookix.rooms.needVerified')"
                @click="openPublishDialog"
              >
                <IconGlobe /> {{ t("qookix.rooms.sharePublic") }}
                <span v-if="!canPublishLobby" class="tc-lock">
                  · {{ t("qookix.rooms.needVerifiedShort") }}
                </span>
              </button>
            </template>
          </div>
        </div>

        <!-- 主机：玩家列表 -->
        <div v-if="tcStateName === 'host-ok' && tcPlayers.length" class="glass tc-players-card">
          <div class="tc-players-head">
            <span class="tc-players-title"><IconUsers /> {{ t('multiplayer.tc.players') }}</span>
            <span class="tc-players-count">{{ tcPlayers.length }}</span>
          </div>
          <div class="tc-players-list">
            <div v-for="(p, i) in tcPlayers" :key="i" class="tc-player-row">
              <span class="tc-player-avatar">{{ (playerName(p) || "?").slice(0, 1) }}</span>
              <div class="tc-player-info">
                <div class="tc-player-name">{{ playerName(p) }}</div>
                <div v-if="playerUuid(p)" class="tc-player-uuid">{{ playerUuid(p) }}</div>
              </div>
              <div v-if="playerDesc(p)" class="tc-player-desc">{{ playerDesc(p) }}</div>
              <span v-if="playerPing(p)" class="tc-player-ping">{{ playerPing(p) }}ms</span>
            </div>
          </div>
        </div>

        <!-- 访客：已加入 -->
        <div v-if="tcStateName === 'guest-ok'" class="glass tc-code-card">
          <div class="tc-code-label"><IconCheck /> {{ t('multiplayer.tc.joined') }}</div>
          <p class="tc-code-hint">
            {{ t('multiplayer.tc.joinHint') }}
            <b class="mono">{{ tcUrl || "127.0.0.1" }}</b>
          </p>
          <div class="tc-code-actions">
            <button class="qx-btn" @click="leaveRoom"><IconClose /> {{ t('multiplayer.tc.leaveRoom') }}</button>
          </div>
        </div>

        <!-- 访客：玩家列表 -->
        <div
          v-if="tcStateName === 'guest-ok' && tcPlayers.length"
          class="glass tc-players-card"
        >
          <div class="tc-players-head">
            <span class="tc-players-title"><IconUsers /> {{ t('multiplayer.tc.players') }}</span>
            <span class="tc-players-count">{{ tcPlayers.length }}</span>
          </div>
          <div class="tc-players-list">
            <div v-for="(p, i) in tcPlayers" :key="i" class="tc-player-row">
              <span class="tc-player-avatar">{{ (playerName(p) || "?").slice(0, 1) }}</span>
              <div class="tc-player-info">
                <div class="tc-player-name">{{ playerName(p) }}</div>
                <div v-if="playerUuid(p)" class="tc-player-uuid">{{ playerUuid(p) }}</div>
              </div>
              <div v-if="playerDesc(p)" class="tc-player-desc">{{ playerDesc(p) }}</div>
              <span v-if="playerPing(p)" class="tc-player-ping">{{ playerPing(p) }}ms</span>
            </div>
          </div>
        </div>

        <!-- 访客：连接中 -->
        <div
          v-if="tcStateName === 'guest-connecting' || tcStateName === 'guest-starting'"
          class="glass tc-scan-card"
        >
          <div class="tc-scan-head">
            <span class="tc-scan-spinner"></span>
            <div>
              <div class="tc-scan-title">{{ tcStateText }}</div>
              <p class="tc-scan-sub">{{ t('multiplayer.tc.connecting') }}</p>
            </div>
          </div>
        </div>

        <!-- 扫描中：引导对局域网开放 -->
        <div
          v-if="tcStateName === 'host-scanning' || tcStateName === 'host-starting'"
          class="glass tc-scan-card"
        >
          <div class="tc-scan-head">
            <span class="tc-scan-spinner"></span>
            <div>
              <div class="tc-scan-title">{{ t('multiplayer.tc.scanning') }}</div>
              <p class="tc-scan-sub">{{ t('multiplayer.tc.scanningSub') }}</p>
            </div>
          </div>
          <div class="tc-scan-steps">
            <div class="tc-scan-step">
              <span class="tc-step-num">1</span>
              <span>{{ t('multiplayer.tc.step1') }}</span>
            </div>
            <div class="tc-scan-step">
              <span class="tc-step-num">2</span>
              <span>{{ t('multiplayer.tc.step2Prefix') }} <b>{{ t('multiplayer.tc.step2Key') }}</b> {{ t('multiplayer.tc.step2Suffix') }}</span>
            </div>
            <div class="tc-scan-step">
              <span class="tc-step-num">3</span>
              <span>{{ t('multiplayer.tc.step3Prefix') }}<b>{{ t('multiplayer.tc.step3Key') }}</b>{{ t('multiplayer.tc.step3Suffix') }}</span>
            </div>
          </div>
          <p class="tc-scan-tip">
            {{ t('multiplayer.tc.scanTip') }}
          </p>
        </div>

        <!-- 空闲：创建 / 加入 -->
        <div v-if="tcStateName === 'waiting'" class="glass tc-room-controls">
          <div class="tc-name-input">
            <label class="tc-field-label">{{ t('multiplayer.tc.playerName') }}</label>
            <n-input
              v-model:value="tcPlayerName"
              :placeholder="t('multiplayer.tc.playerNamePlaceholder')"
              maxlength="16"
              clearable
            />
          </div>
          <div class="tc-control-grid">
            <div class="tc-col">
              <div class="tc-sub-title"><IconBox /> {{ t('multiplayer.tc.createRoom') }}</div>
              <p class="tc-sub-desc">{{ t('multiplayer.tc.createRoomDesc') }}</p>
              <button class="qx-btn qx-btn--primary" :disabled="tcBusy" @click="createRoom">
                <IconPlus /> {{ t('multiplayer.tc.createRoom') }}
              </button>
            </div>
            <div class="tc-divider"></div>
            <div class="tc-col">
              <div class="tc-sub-title"><IconDoorOpen /> {{ t('multiplayer.tc.joinRoom') }}</div>
              <p class="tc-sub-desc">{{ t('multiplayer.tc.joinRoomDesc') }}</p>
              <div class="tc-join-input">
                <n-input
                  v-model:value="tcRoomCode"
                  :placeholder="t('multiplayer.tc.roomCodePlaceholder')"
                  @keyup.enter="joinRoom"
                />
                <button class="qx-btn qx-btn--primary" :disabled="tcBusy" @click="joinRoom">
                  <IconPlay /> {{ t('multiplayer.tc.join') }}
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- 第三方版权标注（AGPL 例外条款要求） -->
      <div class="tc-license">
        {{ t('multiplayer.tc.poweredByPrefix') }}
        <a href="https://github.com/burningtnt/Terracotta" target="_blank" rel="noopener">
          Terracotta
        </a>
        |
        <a href="https://github.com/burningtnt/Terracotta" target="_blank" rel="noopener">
          {{ t('multiplayer.tc.name') }}
        </a>
        {{ t('multiplayer.tc.poweredBySuffix') }}
      </div>
    </template>

    <!-- 好友（可选账号：好友 + 申请 + 黑名单） -->
    <template v-else-if="tab === 'friends'">
      <FriendsPanel @login="openQookixLogin" @joined="onQookixJoined" />
    </template>

    <!-- 内网穿透（frp）：面板隧道 + 本地自定义隧道 -->
    <template v-else-if="tab === 'tunnel'">
      <section class="glass frp-tab">
        <FrpPanel :sub="frpSub" @counts="onFrpCounts" />
      </section>
    </template>

    <QookixLoginDialog ref="qookixLoginRef" />

    <!-- 分享到目录：玩法 / 版本会作为筛选条件；清单用于加入前比对 -->
    <n-modal
      v-model:show="publishOpen"
      preset="card"
      :title="publishVisibility === 'public' ? t('qookix.rooms.publishTitle') : t('qookix.rooms.shareFriendsTitle')"
      style="width: 420px; max-width: 92vw"
      :bordered="false"
    >
      <div class="dialog-body">
        <p class="publish-desc">
          {{
            publishVisibility === "public"
              ? t("qookix.rooms.publishDesc")
              : t("qookix.rooms.shareFriendsDesc")
          }}
        </p>
        <div class="field">
          <label>{{ t("qookix.rooms.titleLabel") }}</label>
          <n-input
            v-model:value="publishTitle"
            :placeholder="t('qookix.rooms.titlePlaceholder')"
            maxlength="60"
          />
        </div>
        <div class="field">
          <label>{{ t("qookix.playType.label") }}</label>
          <n-select
            v-model:value="publishPlayType"
            :options="playTypeOptions"
            :placeholder="t('qookix.playType.none')"
            clearable
          />
        </div>
        <div class="field">
          <label>{{ t("qookix.rooms.versionLabel") }}</label>
          <n-input
            v-model:value="publishVersion"
            :placeholder="t('qookix.rooms.versionPlaceholder')"
            maxlength="32"
          />
        </div>
        <!-- 环境清单：房主顺带分享模组清单，客人加入前能先比对 -->
        <div class="field">
          <label>{{ t("qookix.pack.shareLabel") }}</label>
          <n-switch v-model:value="publishSharePack" />
        </div>
        <div v-if="publishSharePack" class="field">
          <label>{{ t("qookix.pack.instanceLabel") }}</label>
          <n-select
            v-model:value="publishInstanceId"
            :options="packInstanceOptions"
            :placeholder="t('qookix.pack.instancePlaceholder')"
            filterable
          />
          <p class="publish-hint">{{ t("qookix.pack.shareHint") }}</p>
        </div>

        <div class="publish-ops">
          <button class="qx-btn" @click="publishOpen = false">{{ t("common.cancel") }}</button>
          <button class="qx-btn qx-btn--primary" :disabled="shareBusy" @click="confirmPublish">
            {{ t("common.confirm") }}
          </button>
        </div>
      </div>
    </n-modal>

    <!-- 创建服务器（仅名称 / 核心 / 版本）-->
    <n-modal
      :show="dialog !== null"
      preset="card"
      :title="t('multiplayer.dialog.createTitle')"
      style="width: 560px; max-width: 92vw"
      @update:show="(v: boolean) => { if (!v) dialog = null; }"
    >
      <div v-if="dialog" class="dialog-body">
        <div class="field">
          <label>{{ t('multiplayer.dialog.serverName') }}</label>
          <n-input v-model:value="dialog.name" :placeholder="t('multiplayer.dialog.serverNamePlaceholder')" maxlength="40" />
        </div>

        <div class="field">
          <label>{{ t('multiplayer.dialog.serverCore') }}</label>
          <div class="core-grid">
            <button
              v-for="c in CORES"
              :key="c.value"
              class="core-btn"
              :class="{ active: dialog.core === c.value }"
              @click="dialog.core = c.value"
            >
              <span class="core-name">{{ c.label }}</span>
              <span class="core-desc">{{ t(c.desc) }}</span>
            </button>
          </div>
        </div>

        <div class="field">
          <label>{{ t('multiplayer.dialog.gameVersion') }}</label>
          <div class="ver-cats">
            <button
              v-for="c in [
                { key: 'release', label: t('multiplayer.dialog.release') },
                { key: 'snapshot', label: t('multiplayer.dialog.snapshot') },
                { key: 'april', label: t('multiplayer.dialog.april') },
              ]"
              :key="c.key"
              :class="{ active: versionCat === c.key }"
              @click="versionCat = c.key"
            >
              {{ c.label }}
            </button>
          </div>
          <div class="ver-list">
            <button
              v-for="v in filteredVersions"
              :key="v.id"
              class="ver-item"
              :class="{ active: dialog.mc_version === v.id }"
              @click="dialog.mc_version = v.id"
            >
              <span class="ver-id mono">{{ v.id }}</span>
              <span class="ver-type">{{ v.type === "release" || v.type.startsWith("old_") ? t('multiplayer.dialog.releaseShort') : t('multiplayer.dialog.snapshotShort') }}</span>
            </button>
            <div v-if="!filteredVersions.length" class="ver-empty">{{ t('multiplayer.dialog.noVersions') }}</div>
          </div>
        </div>

        <div class="dialog-foot">
          <button class="qx-btn" @click="dialog = null">{{ t('multiplayer.dialog.cancel') }}</button>
          <button class="qx-btn qx-btn--primary" :disabled="!canSave || saving" @click="saveDialog">
            <IconPlus /> {{ saving ? t('multiplayer.dialog.creating') : t('multiplayer.dialog.create') }}
          </button>
        </div>
      </div>
    </n-modal>

    <!-- 删除确认 -->
    <n-modal
      :show="confirmState !== null"
      preset="card"
      :title="t('multiplayer.dialog.deleteTitle')"
      style="width: 420px; max-width: 92vw"
      @update:show="(v: boolean) => { if (!v) confirmState = null; }"
    >
      <div v-if="confirmState" class="confirm-body">
        <p class="confirm-text">{{ t('multiplayer.dialog.confirmDeletePrefix') }}<b>{{ confirmState.name }}</b>{{ t('multiplayer.dialog.confirmDeleteSuffix') }}</p>
        <div class="dialog-foot">
          <button class="qx-btn" @click="confirmState = null">{{ t('multiplayer.dialog.cancel') }}</button>
          <button class="qx-btn qx-btn--danger" @click="doDelete">{{ t('multiplayer.dialog.delete') }}</button>
        </div>
      </div>
    </n-modal>

    <!-- 核心下载进度 -->
    <n-modal
      :show="installing !== null"
      preset="card"
      :title="t('multiplayer.dialog.preparingTitle')"
      style="width: 420px; max-width: 92vw"
      :mask-closable="false"
      :close-on-esc="false"
    >
      <div v-if="installing" class="install-body">
        <div class="install-spin"><IconServer /></div>
        <p class="install-name">{{ installing.name }}</p>
        <p class="install-phase">{{ installing.phase }}</p>
        <div v-if="installHasBytes" class="install-progress">
          <n-progress
            type="line"
            :percentage="installPct"
            :height="8"
            :border-radius="4"
            :show-indicator="false"
            color="#96b5e1"
          />
          <span class="install-bytes">
            {{ fmtBytes(installing.done) }} / {{ fmtBytes(installing.total) }} · {{ installPct }}%
          </span>
        </div>
      </div>
    </n-modal>
  </div>
</template>

<style scoped>
.mp-view {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
/* 主标签与右上角「内网穿透」子标签都挂全局 .qx-seg（高度 / 内边距 / 配色 /
   滑块全部由 components.css 提供），这里只留各自的布局差异：主标签靠左，
   子标签靠右且不参与收缩。改尺寸请改全局类，别在页面里再写一套。 */
.mode-tabs {
  align-self: flex-start;
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: 16px;
}
.server-card {
  padding: 18px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  cursor: pointer;
  position: relative;
  box-shadow: var(--card-shadow);
  transition: transform 0.18s ease, box-shadow 0.18s ease, background 0.12s ease, border-color 0.12s ease;
  /* 卡片逐个进场（同首页 pin-card：translateY 28px 上浮 + 淡入，backwards 让
     未出场的卡片先保持隐藏）。延迟由模板内联 animationDelay 提供。 */
  animation: mp-card-enter 0.34s cubic-bezier(0.22, 0.61, 0.36, 1) backwards;
}
.server-card:hover:not(:active) {
  z-index: 2;
  box-shadow: var(--card-shadow-hover);
}
@keyframes mp-card-enter {
  from {
    transform: translateY(28px);
    opacity: 0;
  }
  to {
    transform: none;
    opacity: 1;
  }
}
/* —— 服务器卡片：与 InstanceCard 同一套结构（card-top + card-foot）与按钮规格 —— */
.card-top {
  display: flex;
  align-items: center;
  gap: 13px;
}
.icon {
  width: 46px;
  height: 46px;
  border-radius: var(--r-md);
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 21px;
  color: var(--accent);
  flex-shrink: 0;
}
.title-wrap {
  flex: 1;
  min-width: 0;
}
.name {
  font-weight: 600;
  font-size: var(--fs-md);
  margin-bottom: 5px;
}
.meta {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: var(--fs-xs);
}
.core-badge {
  font-size: var(--fs-2xs);
  font-weight: 700;
  padding: 1px 7px;
  border-radius: var(--r-xs);
  border: 1px solid;
  letter-spacing: 0.02em;
  flex-shrink: 0;
}
.mc {
  color: var(--text-2);
  font-weight: 600;
}
/* 连接地址：独占一行，不受徽标/版本挤压；数字等宽，换端口时宽度不抖 */
.addr {
  margin-top: 4px;
  font-size: var(--fs-xs);
  line-height: 1.4;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
  min-width: 0;
}
/* 服务器描述（MOTD）：MC 客户端风格的彩色文本区，支持 § / & 格式化码 */
.card-motd {
  font-size: var(--fs-sm);
  line-height: 1.5;
  color: var(--text-2);
  min-width: 0;
  /* 限制最多两行，超出省略，避免长 MOTD 撑高卡片 */
  max-height: 2.6em;
  overflow: hidden;
}
.card-foot {
  display: flex;
  justify-content: space-between;
  align-items: center;
  border-top: 1px solid var(--border);
  padding-top: 11px;
}
.foot-info {
  font-size: var(--fs-xs);
  color: var(--text-3);
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.status {
  font-weight: 600;
  color: var(--text-3);
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.status.on {
  color: var(--success);
}
/* 运行中状态点：雷达式扩散环，让"运行中"有呼吸感 */
.status-dot {
  position: relative;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--success);
  flex-shrink: 0;
}
.status-dot::after {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: 50%;
  background: var(--success);
  opacity: 0.5;
  animation: status-pulse 1.8s ease-out infinite;
}
@keyframes status-pulse {
  0% { transform: scale(1); opacity: 0.5; }
  100% { transform: scale(2.6); opacity: 0; }
}
@media (prefers-reduced-motion: reduce) {
  .status-dot::after { animation: none; opacity: 0; }
}
.actions {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}
.icon-btn {
  width: 32px;
  height: 32px;
  border-radius: var(--r-sm);
  border: 1px solid var(--border);
  background: transparent;
  color: var(--text-2);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  font-size: var(--fs-base);
  transition: all 0.12s;
}
.icon-btn:hover {
  background: var(--w-08);
  color: var(--text-1);
}
.icon-btn.play {
  color: var(--accent);
  border-color: var(--accent-04);
  background: var(--accent-soft);
}
.icon-btn.play.stop {
  color: var(--warn);
  border-color: rgba(224, 168, 90, 0.4);
  background: rgba(224, 168, 90, 0.12);
}
.icon-btn.danger:hover {
  color: var(--danger);
  border-color: var(--danger-50);
}
.empty {
  padding: 48px 24px;
  text-align: center;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--sp-4);
  color: var(--text-3);
}
.empty-icon {
  font-size: 36px;
  opacity: 0.6;
}
.empty p {
  margin: 0;
  font-size: var(--fs-base);
}
.empty .sub {
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.dialog-body {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.field label {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-2);
}
.core-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
  gap: 8px;
}
.core-btn {
  display: flex;
  flex-direction: column;
  gap: 2px;
  align-items: flex-start;
  padding: 9px 12px;
  border-radius: var(--r-md);
  border: 1px solid var(--border);
  background: var(--w-04);
  color: var(--text-2);
  cursor: pointer;
  font-family: inherit;
  text-align: left;
  transition: all 0.13s;
}
.core-btn:hover {
  background: var(--w-08);
}
.core-btn.active {
  background: var(--accent-soft);
  border-color: var(--accent-45);
  color: var(--accent);
}
.core-name {
  font-size: var(--fs-sm);
  font-weight: 700;
}
.core-desc {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.core-btn.active .core-desc {
  color: var(--accent-60);
}
.ver-cats {
  display: flex;
  gap: 4px;
}
.ver-cats button {
  border: none;
  background: var(--w-05);
  color: var(--text-2);
  padding: 7px 16px;
  border-radius: var(--r-sm);
  font-size: var(--fs-sm);
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
}
.ver-cats button.active {
  background: var(--accent-soft);
  color: var(--accent);
}
.ver-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: 240px;
  overflow-y: auto;
  border: 1px solid var(--border);
  border-radius: var(--r-md);
  padding: 4px;
}
.ver-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  border: 1px solid transparent;
  background: transparent;
  color: var(--text-2);
  padding: 7px 10px;
  border-radius: var(--r-sm);
  cursor: pointer;
  font-family: inherit;
  text-align: left;
}
.ver-item:hover {
  background: var(--w-05);
}
.ver-item.active {
  border-color: var(--accent-05);
  background: var(--accent-soft);
  color: var(--accent);
}
.ver-id {
  font-size: var(--fs-xs);
  font-weight: 600;
}
.ver-type {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.ver-empty {
  text-align: center;
  color: var(--text-3);
  font-size: var(--fs-sm);
  padding: 16px 0;
}
.dialog-foot {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}
.confirm-body {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.confirm-text {
  margin: 0;
  font-size: var(--fs-base);
  color: var(--text-2);
  line-height: 1.6;
}
.confirm-text b {
  color: var(--text-1);
}
.install-body {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 18px 0 8px;
}
.install-spin {
  width: 48px;
  height: 48px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: var(--accent-soft);
  color: var(--accent);
  font-size: 24px;
  animation: spin 1.1s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.install-name {
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-1);
}
.install-phase {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--text-3);
}
.install-progress {
  width: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  margin-top: 2px;
}
.install-bytes {
  font-size: var(--fs-xs);
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}

/* ---- 陶瓦联机 ---- */
.tc-download {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.tc-hero {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 20px 24px;
}
.tc-hero-icon {
  width: 52px;
  height: 52px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--r-lg);
  background: var(--accent-soft);
  color: var(--accent);
  font-size: var(--fs-xl);
  overflow: hidden;
}
.tc-icon-img img {
  width: 100%;
  height: 100%;
  object-fit: contain;
  border-radius: var(--r-md);
}
.tc-hero-text {
  flex: 1;
  min-width: 0;
}
.tc-hero-text h2 {
  margin: 0 0 4px;
  font-size: 17px;
}
.tc-hero-text p {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
  line-height: 1.5;
}
.tc-state {
  font-weight: 600;
  color: var(--accent) !important;
}
.tc-hero-ops {
  display: flex;
  gap: 8px;
}
/* 之前 .op 只写了 padding/圆角，没写背景与文字色，按钮退回浏览器默认的白底黑字 */
.tc-hero-ops .op {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 8px 14px;
  font-size: var(--fs-sm);
  font-family: inherit;
  border-radius: var(--r-sm);
  border: 1px solid var(--border);
  background: var(--w-06);
  color: var(--text-1);
  white-space: nowrap;
  cursor: pointer;
  transition: background 0.15s, color 0.15s, border-color 0.15s;
}
.tc-hero-ops .op svg {
  width: 14px;
  height: 14px;
}
.tc-hero-ops .op:hover {
  background: var(--w-10);
}
.tc-hero-ops .op.stop:hover {
  background: var(--danger-16);
  border-color: var(--danger-40);
  color: var(--danger-text);
}
.tc-hero-launch {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 14px;
  border: 1px solid var(--accent-45);
  border-radius: var(--r-sm);
  background: var(--accent-soft);
  color: var(--accent);
  font-size: var(--fs-sm);
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
  transition: all 0.15s;
  flex-shrink: 0;
}
.tc-hero-launch:hover:not(:disabled) {
  background: var(--accent);
  color: #fff;
}
.tc-hero-launch:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
.tc-hero-launch-icon {
  font-size: 16px;
}
.tc-dl-card {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 20px 24px;
}
.tc-dl-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-md);
  font-weight: 700;
  color: var(--text-1);
}
.tc-dl-desc {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--text-2);
  line-height: 1.6;
}
.tc-dl-actions {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
}
.tc-dl-progress {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.tc-dl-bar {
  height: 12px;
  border-radius: var(--r-xs);
  background: var(--w-07);
  border: 1px solid var(--border);
  overflow: hidden;
}
.tc-dl-fill {
  height: 100%;
  border-radius: var(--r-xs);
  background: linear-gradient(90deg, var(--accent), var(--accent-2, var(--accent)));
  transition: width 0.2s ease;
}
.tc-dl-text {
  font-size: var(--fs-xs);
  color: var(--text-2);
}
.tc-hint {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.tc-hint code {
  padding: 2px 6px;
  border-radius: var(--r-xs);
  background: var(--w-06);
  border: 1px solid var(--border);
  font-size: var(--fs-2xs);
}
.tc-license {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  position: fixed;
  left: 50%;
  transform: translateX(-50%);
  bottom: 12px;
  z-index: 40;
  padding: 5px 14px;
  border-radius: var(--r-pill);
  background: var(--panel);
  border: 1px solid var(--border);
  backdrop-filter: blur(var(--glass-blur, 8px));
  font-size: var(--fs-2xs);
  color: var(--text-3);
  text-align: center;
  white-space: nowrap;
  box-shadow: 0 4px 16px var(--k-20);
}
.tc-license a {
  color: var(--text-2);
  text-decoration: none;
}
.tc-license a:hover {
  color: var(--accent);
  text-decoration: underline;
}
.tc-code-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
  padding: 20px 24px;
  text-align: center;
}
.tc-code-label {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text-1);
}
.tc-code-value {
  font-size: 30px;
  font-weight: 700;
  letter-spacing: 0.12em;
  color: var(--accent);
  background: var(--accent-soft);
  padding: 12px 22px;
  border-radius: var(--r-lg);
}
.tc-code-hint {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.tc-code-hint .mono {
  color: var(--text-1);
}
.tc-players-card {
  padding: 18px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.tc-players-head {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text-1);
}
.tc-players-count {
  min-width: 20px;
  height: 20px;
  padding: 0 6px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--r-md);
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--accent);
  background: var(--accent-soft);
}
.tc-players-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.tc-player-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border: 1px solid var(--border);
  border-radius: var(--r-md);
  background: var(--panel);
}
.tc-player-avatar {
  width: 32px;
  height: 32px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--r-sm);
  font-weight: 600;
  color: var(--accent);
  background: var(--accent-soft);
}
.tc-player-info {
  flex: 1;
  min-width: 0;
}
.tc-player-name {
  font-size: var(--fs-sm);
  font-weight: 500;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tc-player-uuid {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tc-player-desc {
  flex-shrink: 0;
  font-size: var(--fs-xs);
  color: var(--text-2);
}
.tc-player-ping {
  flex-shrink: 0;
  font-size: var(--fs-xs);
  font-weight: 500;
  color: var(--accent);
  background: var(--accent-soft);
  padding: 2px 8px;
  border-radius: var(--r-md);
}
.tc-scan-card {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 24px;
}
.tc-scan-head {
  display: flex;
  align-items: center;
  gap: 14px;
}
.tc-scan-spinner {
  width: 22px;
  height: 22px;
  flex-shrink: 0;
  border: 3px solid var(--accent-soft);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: tc-spin 0.8s linear infinite;
}
@keyframes tc-spin {
  to {
    transform: rotate(360deg);
  }
}
.tc-scan-title {
  font-size: var(--fs-base);
  font-weight: 600;
  color: var(--text-1);
}
.tc-scan-sub {
  margin: 2px 0 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.tc-scan-steps {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px;
  border: 1px solid var(--border);
  border-radius: var(--r-md);
  background: var(--panel);
}
.tc-scan-step {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: var(--fs-sm);
  color: var(--text-2);
}
.tc-step-num {
  width: 20px;
  height: 20px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--accent);
  background: var(--accent-soft);
}
.tc-scan-step b {
  color: var(--text-1);
}
.tc-scan-tip {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.tc-code-actions {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
  justify-content: center;
}
.tc-room-controls {
  padding: 20px 24px;
}
.tc-room-controls > .tc-name-input {
  margin-bottom: 18px;
  max-width: 320px;
}
.tc-control-grid {
  display: grid;
  grid-template-columns: 1fr 1px 1fr;
  gap: 20px;
  align-items: stretch;
}
.tc-col {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 8px;
}
.tc-sub-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-base);
  font-weight: 700;
  color: var(--text-1);
}
.tc-sub-desc {
  margin: 0 0 4px;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.tc-join-input {
  display: flex;
  gap: 8px;
  width: 100%;
}
.tc-join-input :deep(.n-input) {
  flex: 1;
}
.tc-name-input {
  width: 100%;
}
.tc-field-label {
  display: block;
  margin-bottom: 4px;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.tc-divider {
  width: 1px;
  background: var(--border);
}

/* 公开到大厅弹窗 */
.publish-desc {
  margin: 0 0 4px;
  font-size: var(--fs-xs);
  line-height: 1.6;
  color: var(--text-3);
}
.publish-ops {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 4px;
}

/* 顶栏：主标签靠左，子页签靠右，同一行 */
.mp-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
/* 子标签只留布局差异；按钮 / 指示器规格由全局 .qx-seg 提供 */
.mp-subtabs {
  flex: none;
}
/* 不在穿透页时隐藏但仍占位：宽度保持不变，滑块不用重新测 */
.mp-subtabs.hidden {
  visibility: hidden;
  pointer-events: none;
}
/* 数量徽标沿用全局 .qx-badge，这里只压成等宽数字块；
   数量为 0 时模板直接不渲染，不再用隐藏元素占位留空白 */
.mp-sub-count {
  min-width: var(--sp-6);
  justify-content: center;
  padding: 0 var(--sp-1);
}

/* 内网穿透标签页：与好友/房间面板同一套卡片内边距 */
.frp-tab {
  padding: var(--sp-6) var(--sp-7);
  border-radius: var(--r-lg);
  min-width: 0;
}

/* 联机页签上的待处理角标 */
.tab-badge {
  margin-left: 4px;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  border-radius: var(--r-pill);
  background: var(--danger, #e5534b);
  color: #fff;
  font-size: var(--fs-2xs);
  line-height: 16px;
  text-align: center;
}

/* 房主卡片：分享到目录（做成一行状态 + 文字按钮，避免和主操作抢视觉） */
.tc-share-row {
  display: flex;
  align-items: center;
  justify-content: center;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 14px;
  padding-top: 14px;
  border-top: 1px solid var(--border);
}
.tc-share-dot {
  width: 7px;
  height: 7px;
  flex: none;
  border-radius: 50%;
  background: var(--success, #4ec9a0);
  box-shadow: 0 0 0 3px rgba(78, 201, 160, 0.18);
}
.tc-share-dot.pub {
  background: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-08, rgba(232, 154, 75, 0.16));
}
.tc-share-text {
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.tc-share-sep {
  width: 1px;
  height: 12px;
  background: var(--border);
}
.tc-share-link {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border: none;
  background: none;
  padding: 0;
  font-size: var(--fs-xs);
  font-family: inherit;
  color: var(--text-2);
  white-space: nowrap;
  cursor: pointer;
  transition: color var(--dur-sm, 0.15s) var(--ease-out, ease);
}
.tc-share-link svg {
  width: 13px;
  height: 13px;
}
.tc-share-link:hover:not(:disabled) {
  color: var(--accent);
}
.tc-share-link:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}
/* 未绑定正版时大厅入口降权：仍然可见可点，点了会说明缺什么 */
.tc-share-link.locked {
  color: var(--text-3);
}
.tc-lock {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
</style>
