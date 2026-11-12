<script setup lang="ts">
/**
 * 内网穿透（MohuaFrp）面板内容。
 *
 * 两类隧道分两个区块，各自一卡一隧道：
 * - MohuaFrp 隧道：在 MohuaFrp 网页上创建，这里只负责登录与启停；
 * - 本地隧道：直接在本机生成 frpc 配置（认证 token 用登录态），可自由改端口。
 */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { NInput, NModal, NSelect, useMessage } from "naive-ui";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../api";
import type { FrpTunnel, FrpUser } from "../types";
import { IconExternal, IconGlobe, IconRefresh } from "./icons";

const { t } = useI18n();
const message = useMessage();
const SITE = "https://frp.mhjz1.cn";

const loading = ref(false);
const user = ref<FrpUser | null>(null);
const loggedIn = ref(false);
const tunnels = ref<FrpTunnel[]>([]);
const running = ref<string[]>([]);

const authCode = ref("");
const authUri = ref("");
const polling = ref(false);
let pollTimer: ReturnType<typeof setInterval> | null = null;

const runningSet = computed(() => new Set(running.value));

interface LocalTunnel {
  name: string;
  protocol: string;
  local_port: number;
  remote_port: number;
  host: string;
  port: number;
}
const localTunnels = ref<LocalTunnel[]>([]);
const editing = ref<LocalTunnel | null>(null);
const showForm = ref(false);
/// 子页签由父级（联机页顶栏）控制，这里只负责按当前值渲染
const props = withDefaults(defineProps<{ sub?: "panel" | "local" }>(), { sub: "panel" });
const sub = computed(() => props.sub);
/// 把两边数量抛给父级，画在顶栏的子页签上
const emit = defineEmits<{ counts: [panel: number, local: number] }>();
watch(
  [() => tunnels.value.length, () => localTunnels.value.length],
  () => emit("counts", tunnels.value.length, localTunnels.value.length),
  { immediate: true },
);

const protoOptions = [
  { label: "TCP", value: "tcp" },
  { label: "UDP", value: "udp" },
];

const logOpen = ref(false);
const logName = ref("");
const logText = ref("");
const logEl = ref<HTMLElement | null>(null);
let logTimer: ReturnType<typeof setInterval> | null = null;

/// 读一次日志；有变化才更新并滚到底，避免无谓重渲染
async function readLog(silent = false) {
  const name = logName.value;
  if (!name) return;
  try {
    const text = await api.frpLogs(name, 300);
    if (text === logText.value) return;
    logText.value = text;
    await nextTick();
    if (logEl.value) logEl.value.scrollTop = logEl.value.scrollHeight;
  } catch (e) {
    if (!silent) logText.value = String(e);
  }
}

function stopLogPolling() {
  if (logTimer) {
    clearInterval(logTimer);
    logTimer = null;
  }
}

async function showLog(name: string) {
  logName.value = name;
  logOpen.value = true;
  logText.value = t("frp.logLoading");
  await readLog();
}

// 打开期间每秒刷新一次；关掉就停，别空转
watch(logOpen, (v) => {
  stopLogPolling();
  if (v) logTimer = setInterval(() => void readLog(true), 1000);
});

function stopPolling() {
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
  polling.value = false;
}

async function refresh() {
  loading.value = true;
  try {
    const s = await api.frpStatus();
    loggedIn.value = s.loggedIn;
    user.value = s.user ?? null;
    if (s.loggedIn) await Promise.all([loadTunnels(), loadLocal()]);
  } catch (e) {
    message.error(String(e));
  } finally {
    loading.value = false;
  }
}

async function loadTunnels() {
  try {
    const r = await api.frpTunnels();
    tunnels.value = r.tunnels ?? [];
    running.value = r.running ?? [];
  } catch (e) {
    message.error(String(e));
  }
}

async function loadLocal() {
  try {
    localTunnels.value = await api.frpLocalList();
  } catch {
    localTunnels.value = [];
  }
}

async function startAuth() {
  loading.value = true;
  try {
    const d = await api.frpLoginStart();
    authCode.value = d.userCode;
    authUri.value = d.uri;
    const every = Math.max(5, d.interval || 5) * 1000;
    stopPolling();
    polling.value = true;
    pollTimer = setInterval(async () => {
      try {
        const r = await api.frpLoginPoll();
        if (r.status === "authorized") {
          stopPolling();
          authCode.value = "";
          message.success(t("frp.loginOk"));
          await refresh();
        }
      } catch (e) {
        stopPolling();
        message.error(String(e));
      }
    }, every);
  } catch (e) {
    message.error(String(e));
  } finally {
    loading.value = false;
  }
}

async function start(tn: FrpTunnel) {
  try {
    const r = await api.frpStart(tn.id);
    await loadTunnels();
    if (!running.value.includes(r.name)) {
      // 起来了又立刻退出：多半是端口不允许或 token 不对，直接引导看日志
      message.warning(t("frp.processExited"));
      await showLog(r.name);
      return;
    }
    message.success(t("frp.started", { name: r.name }));
  } catch (e) {
    message.error(String(e));
  }
}

async function stopName(name: string) {
  try {
    await api.frpStop(name);
    message.success(t("frp.stopped"));
    await Promise.all([loadTunnels(), loadLocal()]);
  } catch (e) {
    message.error(String(e));
  }
}

async function logout() {
  try {
    await api.frpLogout();
    loggedIn.value = false;
    user.value = null;
    tunnels.value = [];
    localTunnels.value = [];
    running.value = [];
    message.success(t("frp.loggedOut"));
  } catch (e) {
    message.error(String(e));
  }
}

function newLocal() {
  editing.value = {
    name: "",
    protocol: "tcp",
    local_port: 25565,
    remote_port: 30000,
    host: tunnels.value[0]?.node_hostname ?? "",
    port: tunnels.value[0]?.node_port ?? 7000,
  };
  showForm.value = true;
}

function editLocal(tn: LocalTunnel) {
  editing.value = { ...tn };
  showForm.value = true;
}

async function saveLocal() {
  const tn = editing.value;
  if (!tn) return;
  if (!tn.name.trim()) {
    message.warning(t("frp.needName"));
    return;
  }
  try {
    await api.frpLocalSave(tn);
    showForm.value = false;
    editing.value = null;
    message.success(t("frp.saved"));
    await loadLocal();
  } catch (e) {
    message.error(String(e));
  }
}

async function startLocal(tn: LocalTunnel) {
  try {
    await api.frpLocalStart(tn.name);
    await Promise.all([loadTunnels(), loadLocal()]);
    if (!running.value.includes(tn.name)) {
      message.warning(t("frp.processExited"));
      await showLog(tn.name);
      return;
    }
    message.success(t("frp.started", { name: tn.name }));
  } catch (e) {
    message.error(String(e));
  }
}

async function removeLocal(tn: LocalTunnel) {
  await api.frpLocalDelete(tn.name);
  await loadLocal();
}

function statusText(s: string): string {
  const map: Record<string, string> = {
    active: "frp.statusActive",
    expired: "frp.statusExpired",
    disabled: "frp.statusDisabled",
    pending: "frp.statusPending",
    deleted: "frp.statusDeleted",
  };
  return map[s] ? t(map[s]) : s;
}

function remoteOf(tn: FrpTunnel): string {
  if (tn.custom_domain) return tn.custom_domain;
  const host = tn.node_hostname || "";
  return tn.remote_port ? `${host}:${tn.remote_port}` : host;
}

onMounted(() => void refresh());
onUnmounted(() => {
  stopPolling();
  stopLogPolling();
});
</script>

<template>
  <div class="frp-root">
    <!-- 未登录：设备授权 -->
    <section v-if="!loggedIn" class="frp-card">
      <header class="frp-card-head">
        <span class="frp-card-title"><IconGlobe /> {{ t("frp.brand") }}</span>
        <span class="frp-status">{{ t("frp.notLoggedIn") }}</span>
        <span class="frp-spacer"></span>
        <button class="frp-btn" @click="openUrl(SITE)">
          <IconExternal /> {{ t("frp.openSite") }}
        </button>
      </header>
      <div class="frp-card-body">
        <p class="frp-hint">{{ t("frp.loginHint") }}</p>
        <div v-if="authCode" class="frp-auth">
          <div class="frp-code">{{ authCode }}</div>
          <div class="frp-auth-ops">
            <button class="frp-btn primary" @click="openUrl(authUri)">
              <IconExternal /> {{ t("frp.openAuth") }}
            </button>
            <span v-if="polling" class="frp-wait">{{ t("frp.waiting") }}</span>
          </div>
          <p class="frp-hint">{{ t("frp.authHint") }}</p>
        </div>
        <button v-else class="frp-btn primary" :disabled="loading" @click="startAuth">
          {{ loading ? t("frp.gettingCode") : t("frp.getCode") }}
        </button>
      </div>
    </section>

    <template v-else>
      <!-- 账号条 -->
      <section class="frp-card">
        <header class="frp-card-head">
          <span class="frp-card-title"><IconGlobe /> {{ t("frp.brand") }}</span>
          <span class="frp-status on">{{ t("frp.loggedIn") }}</span>
          <span class="frp-spacer"></span>
          <span class="frp-account">{{ user?.username || "" }}</span>
          <button class="frp-btn" @click="openUrl(SITE)">
            <IconExternal /> {{ t("frp.openSite") }}
          </button>
          <button class="frp-icon-btn" :title="t('frp.refresh')" @click="loadTunnels">
            <IconRefresh />
          </button>
          <button class="frp-btn" @click="logout">{{ t("frp.logout") }}</button>
        </header>
      </section>

      <!-- 两个子页签的内容带淡入淡出，切换时不硬切 -->
      <transition name="frp-fade" mode="out-in">
      <!-- MohuaFrp 隧道 -->
      <section v-if="sub === 'panel'" key="panel" class="frp-card">
        <header class="frp-card-head">
          <span class="frp-card-title">{{ t("frp.panelTunnels") }}</span>
          <span class="frp-count">{{ tunnels.length }}</span>
        </header>
        <p class="frp-hint">{{ t("frp.panelHint") }}</p>
        <p v-if="!tunnels.length" class="frp-empty">{{ t("frp.panelEmpty") }}</p>
        <div v-else class="frp-grid">
          <article v-for="tn in tunnels" :key="tn.id" class="frp-tun">
            <div class="frp-tun-top">
              <span class="frp-tun-name">{{ tn.name }}</span>
              <span class="frp-status" :class="tn.status">{{ statusText(tn.status) }}</span>
            </div>
            <div class="frp-tags">
              <span class="frp-tag proto">{{ tn.protocol }}</span>
              <span v-if="tn.node_name" class="frp-tag">{{ tn.node_name }}</span>
            </div>
            <div class="frp-flow">
              <span class="frp-flow-local">127.0.0.1:{{ tn.local_port }}</span>
              <span class="frp-flow-arrow">→</span>
              <span class="frp-flow-remote">{{ remoteOf(tn) }}</span>
            </div>
            <div class="frp-tun-foot">
              <span v-if="runningSet.has(tn.name)" class="frp-running">
                <i></i> {{ t("frp.running") }}
              </span>
              <span v-else class="frp-idle">{{ t("frp.idle") }}</span>
              <span class="frp-spacer"></span>
              <button class="frp-icon-text" @click="showLog(tn.name)">{{ t("frp.logs") }}</button>
              <button
                v-if="runningSet.has(tn.name)"
                class="frp-btn danger"
                @click="stopName(tn.name)"
              >
                {{ t("frp.stop") }}
              </button>
              <button
                v-else
                class="frp-btn primary"
                :disabled="tn.status !== 'active'"
                @click="start(tn)"
              >
                {{ t("frp.start") }}
              </button>
            </div>
          </article>
        </div>
      </section>

      <!-- 本地隧道 -->
      <section v-else key="local" class="frp-card">
        <header class="frp-card-head">
          <span class="frp-card-title">{{ t("frp.localTunnels") }}</span>
          <span class="frp-count">{{ localTunnels.length }}</span>
          <span class="frp-spacer"></span>
          <button class="frp-btn" @click="newLocal">{{ t("frp.create") }}</button>
        </header>
        <p class="frp-hint">{{ t("frp.localHint") }}</p>

        <div v-if="showForm && editing" class="frp-form">
          <div class="frp-field">
            <label>{{ t("frp.name") }}</label>
            <n-input
              v-model:value="editing.name"
              size="small"
              :placeholder="t('frp.namePlaceholder')"
            />
          </div>
          <div class="frp-field narrow">
            <label>{{ t("frp.protocol") }}</label>
            <n-select v-model:value="editing.protocol" size="small" :options="protoOptions" />
          </div>
          <div class="frp-field narrow">
            <label>{{ t("frp.localPort") }}</label>
            <n-input
              :value="String(editing.local_port)"
              size="small"
              @update:value="(v: string) => (editing!.local_port = Number(v) || 0)"
            />
          </div>
          <div class="frp-field narrow">
            <label>{{ t("frp.remotePort") }}</label>
            <n-input
              :value="String(editing.remote_port)"
              size="small"
              @update:value="(v: string) => (editing!.remote_port = Number(v) || 0)"
            />
          </div>
          <div class="frp-field">
            <label>{{ t("frp.nodeHost") }}</label>
            <n-input v-model:value="editing.host" size="small" placeholder="node1.frp.mhjz1.cn" />
          </div>
          <div class="frp-field narrow">
            <label>{{ t("frp.nodePort") }}</label>
            <n-input
              :value="String(editing.port)"
              size="small"
              @update:value="(v: string) => (editing!.port = Number(v) || 0)"
            />
          </div>
          <div class="frp-form-ops">
            <button class="frp-btn" @click="showForm = false">{{ t("frp.cancel") }}</button>
            <button class="frp-btn primary" @click="saveLocal">{{ t("frp.save") }}</button>
          </div>
        </div>

        <p v-if="!localTunnels.length" class="frp-empty">{{ t("frp.localEmpty") }}</p>
        <div v-else class="frp-grid">
          <article v-for="tn in localTunnels" :key="tn.name" class="frp-tun">
            <div class="frp-tun-top">
              <span class="frp-tun-name">{{ tn.name }}</span>
              <span class="frp-status">{{ tn.protocol.toUpperCase() }}</span>
            </div>
            <div class="frp-flow">
              <span class="frp-flow-local">127.0.0.1:{{ tn.local_port }}</span>
              <span class="frp-flow-arrow">→</span>
              <span class="frp-flow-remote">{{ tn.host }}:{{ tn.remote_port }}</span>
            </div>
            <div class="frp-tun-foot">
              <span v-if="runningSet.has(tn.name)" class="frp-running">
                <i></i> {{ t("frp.running") }}
              </span>
              <span v-else class="frp-idle">{{ t("frp.idle") }}</span>
              <span class="frp-spacer"></span>
              <button class="frp-icon-text" @click="showLog(tn.name)">{{ t("frp.logs") }}</button>
              <button class="frp-icon-text" @click="editLocal(tn)">{{ t("frp.edit") }}</button>
              <button class="frp-icon-text danger" @click="removeLocal(tn)">
                {{ t("frp.remove") }}
              </button>
              <button
                v-if="runningSet.has(tn.name)"
                class="frp-btn danger"
                @click="stopName(tn.name)"
              >
                {{ t("frp.stop") }}
              </button>
              <button v-else class="frp-btn primary" @click="startLocal(tn)">
                {{ t("frp.start") }}
              </button>
            </div>
          </article>
        </div>
      </section>
      </transition>
    </template>

    <!-- 日志：frpc 的 stdout/stderr，端口被拒、token 错误都会打在这里 -->
    <n-modal
      v-model:show="logOpen"
      preset="card"
      :title="t('frp.logTitle', { name: logName })"
      style="width: 720px; max-width: 94vw"
      :bordered="false"
    >
      <div class="frp-log-wrap">
        <pre ref="logEl" class="frp-log">{{ logText }}</pre>
        <div class="frp-log-ops">
          <button class="frp-btn" @click="showLog(logName)">{{ t("frp.refresh") }}</button>
          <button class="frp-btn primary" @click="logOpen = false">{{ t("frp.close") }}</button>
        </div>
      </div>
    </n-modal>
  </div>
</template>

<style scoped>
.frp-root {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
}

/**
 * 区块本身**不是**卡片：外层已经有一层 glass，再套一层就是三层嵌套。
 * 这里只当排版容器，真正有卡面的是下面的 .frp-tun。
 */
.frp-card {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-width: 0;
}
/* 子页签切换：淡入 + 轻微上浮，别硬切 */
.frp-fade-enter-active,
.frp-fade-leave-active {
  transition: opacity 0.18s ease, transform 0.18s ease;
}
.frp-fade-enter-from {
  opacity: 0;
  transform: translateY(5px);
}
.frp-fade-leave-to {
  opacity: 0;
  transform: translateY(-5px);
}

/* 区块之间用一条细线分隔，代替整块底色。
   只在「后一块」画上边框，第一块不要再画下边框，否则接缝处会叠成两条线。 */
.frp-card + .frp-card {
  margin-top: 4px;
  padding-top: 14px;
  border-top: 1px solid var(--border);
}
.frp-card-head {
  display: flex;
  align-items: center;
  gap: 8px;
}
.frp-card-title {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-1);
}
.frp-card-title svg {
  width: 14px;
  height: 14px;
  color: var(--accent);
}
.frp-count {
  min-width: 18px;
  height: 18px;
  padding: 0 5px;
  border-radius: 999px;
  background: var(--w-06);
  color: var(--text-3);
  font-size: 10px;
  line-height: 18px;
  text-align: center;
}
.frp-spacer {
  flex: 1;
}

/* 子页签：贴在右侧，只做切换，不抢主视觉 */
.frp-subtabs {
  display: flex;
  justify-content: flex-end;
  gap: 4px;
  margin: -2px 0 2px;
}
.frp-subtabs button {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 26px;
  padding: 0 10px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text-3);
  font-size: 11px;
  font-family: inherit;
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
}
.frp-subtabs button:hover {
  color: var(--text-1);
  background: var(--w-06);
}
.frp-subtabs button.on {
  color: var(--accent);
  background: var(--accent-08, rgba(232, 154, 75, 0.12));
}
.frp-account {
  font-size: 11px;
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 180px;
}

.frp-hint {
  margin: 0;
  font-size: 11px;
  line-height: 1.65;
  color: var(--text-3);
}
.frp-empty {
  margin: 0;
  padding: 18px 0;
  text-align: center;
  font-size: 12px;
  color: var(--text-3);
}

/* 一隧道一卡 */
.frp-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
  gap: 10px;
}
.frp-tun {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 14px;
  border-radius: 12px;
  border: 1px solid var(--border);
  /* 比外层 glass 亮一档，让「这一张是实体卡」一眼可辨 */
  background: var(--w-06);
  min-width: 0;
  transition: border-color 0.15s ease, background 0.15s ease;
}
.frp-tun:hover {
  border-color: var(--border-strong, rgba(255, 255, 255, 0.16));
  background: var(--w-10);
}
.frp-tun-top {
  display: flex;
  align-items: center;
  gap: 8px;
}
.frp-tun-name {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.frp-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
}
.frp-tag {
  font-size: 10px;
  padding: 2px 7px;
  border-radius: 6px;
  background: var(--bg-2, rgba(255, 255, 255, 0.04));
  color: var(--text-3);
}
.frp-tag.proto {
  color: var(--accent);
  background: var(--accent-08, rgba(232, 154, 75, 0.12));
}
.frp-flow {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  font-family: ui-monospace, Consolas, monospace;
  color: var(--text-3);
  overflow: hidden;
}
.frp-flow-local,
.frp-flow-remote {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.frp-flow-arrow {
  flex: none;
  color: var(--text-3);
  opacity: 0.6;
}
.frp-flow-remote {
  color: var(--text-2);
}
.frp-tun-foot {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 2px;
}
.frp-running {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  color: var(--success, #4ec9a0);
}
.frp-running i {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--success, #4ec9a0);
  box-shadow: 0 0 0 3px rgba(78, 201, 160, 0.16);
}
.frp-idle {
  font-size: 11px;
  color: var(--text-3);
}

.frp-status {
  flex: none;
  font-size: 10px;
  padding: 2px 7px;
  border-radius: 999px;
  border: 1px solid var(--border);
  color: var(--text-3);
}
.frp-status.active,
.frp-status.on {
  color: var(--success, #4ec9a0);
  border-color: rgba(78, 201, 160, 0.4);
}
.frp-status.expired,
.frp-status.disabled {
  color: var(--danger, #e5534b);
  border-color: rgba(229, 83, 75, 0.35);
}

/* 日志弹窗 */
.frp-log-wrap {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.frp-log {
  margin: 0;
  max-height: 420px;
  overflow: auto;
  padding: 12px;
  border-radius: 10px;
  background: var(--w-06, rgba(255, 255, 255, 0.06));
  color: var(--text-2);
  font-family: ui-monospace, Consolas, monospace;
  font-size: 11px;
  line-height: 1.65;
  white-space: pre-wrap;
  word-break: break-all;
  /* 桌面端全局禁用了选择，日志必须能选中复制 */
  user-select: text;
  -webkit-user-select: text;
  cursor: text;
}
.frp-log-ops {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

/* 登录区 */
.frp-auth {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.frp-code {
  font-family: ui-monospace, Consolas, monospace;
  font-size: 26px;
  letter-spacing: 3px;
  text-align: center;
  padding: 14px 0;
  border-radius: 10px;
  background: var(--w-06);
  color: var(--accent);
}
.frp-auth-ops {
  display: flex;
  align-items: center;
  gap: 10px;
}
.frp-wait {
  font-size: 12px;
  color: var(--text-3);
}

/* 表单 */
.frp-form {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  padding: 12px;
  border-radius: 10px;
  border: 1px solid var(--border);
  background: var(--w-04);
}
.frp-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  width: 180px;
}
.frp-field.narrow {
  width: 110px;
}
.frp-field label {
  font-size: 11px;
  color: var(--text-3);
}
.frp-form-ops {
  display: flex;
  align-items: flex-end;
  gap: 8px;
  margin-left: auto;
}

/* 按钮 */
.frp-btn {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  height: 26px;
  padding: 0 10px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--w-06);
  color: var(--text-1);
  font-size: 11px;
  font-family: inherit;
  white-space: nowrap;
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
}
.frp-btn svg {
  width: 12px;
  height: 12px;
}
.frp-btn:hover:not(:disabled) {
  background: var(--w-10);
}
.frp-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.frp-btn.primary {
  border-color: transparent;
  background: var(--accent);
  color: #fff;
}
.frp-btn.danger {
  color: var(--danger, #e5534b);
}
.frp-icon-text {
  flex: none;
  border: none;
  background: none;
  padding: 0 2px;
  font-size: 11px;
  font-family: inherit;
  color: var(--text-3);
  white-space: nowrap;
  cursor: pointer;
}
.frp-icon-text:hover {
  color: var(--accent);
}
.frp-icon-text.danger:hover {
  color: var(--danger, #e5534b);
}
.frp-icon-btn {
  flex: none;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}
.frp-icon-btn svg {
  width: 13px;
  height: 13px;
}
.frp-icon-btn:hover {
  color: var(--accent);
  border-color: var(--accent-35, rgba(232, 154, 75, 0.35));
}
</style>
