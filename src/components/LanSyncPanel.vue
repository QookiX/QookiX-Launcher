<script setup lang="ts">
/**
 * 局域网同步面板 —— 主题对齐版。
 *
 * 设计语言：与 SettingsView 的 .card.glass + .tc-scan-steps 一致，
 * 全部使用 --panel / --border / --text-* / --accent / --success / --danger 变量。
 */
import { ref, onMounted, onUnmounted, computed } from "vue";
import QRCode from "qrcode";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api } from "../api";
import { useI18n } from "vue-i18n";
import { useMessage } from "naive-ui";
import type { LanSyncState } from "../types";

const { t } = useI18n();
const message = useMessage();

const running = ref(false);
const state = ref<LanSyncState | null>(null);
const qrDataUrl = ref("");
const loading = ref(false);
const error = ref("");

// ── 传输进度（2026-10-30 新增）────────────────────────────────────────────
// 原来这一页只有二维码和地址：手机在拉几百 MB 的实例时，电脑端屏幕上
// 一个字都不动，用户完全不知道有没有在传、传到哪了。
interface Transfer {
  id: string;
  direction: "download" | "upload";
  instanceId: string;
  instanceName: string;
  phase: "prepare" | "pack" | "send" | "done" | "error";
  bytesDone: number;
  bytesTotal: number;
  file: string;
  message: string;
}

const active = ref<Transfer | null>(null);
const recent = ref<(Transfer & { endedAt: number })[]>([]);
const speedBps = ref(0);
let unlisten: UnlistenFn | null = null;
let lastTick = { at: 0, bytes: 0 };
/// 最后一次收到事件的时间：手机中途断开时不会有 done/error，
/// 靠这个把「准备中」的卡片清掉，不然它会一直挂在那
let lastEventAt = 0;
let staleTimer: number | undefined;

function humanBytes(n: number): string {
  if (!n || n < 0) return "0 B";
  const u = ["B", "KB", "MB", "GB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${u[i]}`;
}

const activePercent = computed(() => {
  const a = active.value;
  if (!a || a.bytesTotal <= 0) return 0;
  return Math.min(100, Math.round((a.bytesDone / a.bytesTotal) * 100));
});
const activeEta = computed(() => {
  const a = active.value;
  if (!a || a.bytesTotal <= 0 || a.bytesDone >= a.bytesTotal || speedBps.value <= 0) return "";
  const s = Math.max(1, Math.round((a.bytesTotal - a.bytesDone) / speedBps.value));
  return s >= 60 ? `${Math.floor(s / 60)} 分 ${s % 60} 秒` : `${s} 秒`;
});

function phaseText(p: Transfer): string {
  switch (p.phase) {
    case "prepare":
      return t("settings.lanSync.phasePrepare");
    case "pack":
      return t("settings.lanSync.phasePack");
    case "send":
      return t("settings.lanSync.phaseSend");
    case "done":
      return t("settings.lanSync.phaseDone");
    case "error":
      return t("settings.lanSync.phaseError");
  }
  return "";
}

function onTransfer(payload: Transfer) {
  lastEventAt = Date.now();
  if (payload.phase === "done" || payload.phase === "error") {
    recent.value = [{ ...payload, endedAt: Date.now() }, ...recent.value].slice(0, 5);
    if (active.value?.id === payload.id) {
      active.value = null;
      speedBps.value = 0;
    }
    return;
  }
  if (active.value?.id !== payload.id) {
    speedBps.value = 0;
    lastTick = { at: Date.now(), bytes: payload.bytesDone };
  }
  const now = Date.now();
  const dt = (now - lastTick.at) / 1000;
  if (dt >= 0.4) {
    const delta = payload.bytesDone - lastTick.bytes;
    const inst = delta > 0 ? delta / dt : 0;
    speedBps.value = speedBps.value > 0 ? speedBps.value * 0.6 + inst * 0.4 : inst;
    lastTick = { at: now, bytes: payload.bytesDone };
  }
  active.value = payload;
}

const multiAdapter = computed(() => (state.value?.ips?.length ?? 0) > 1);

async function renderQR(url: string) {
  qrDataUrl.value = await QRCode.toDataURL(url, {
    width: 160,
    margin: 1,
    color: { dark: "#14171f", light: "#ffffff" },
  });
}

async function applyState(s: LanSyncState) {
  state.value = s;
  await renderQR(s.qrUrl);
}

async function start() {
  loading.value = true;
  error.value = "";
  try {
    const s = await api.lanSyncStart();
    running.value = true;
    await applyState(s);
  } catch (e: any) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function stop() {
  loading.value = true;
  try {
    await api.lanSyncStop();
    running.value = false;
    state.value = null;
    qrDataUrl.value = "";
  } catch (e: any) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function pickIp(ip: string) {
  if (!state.value || ip === state.value.ip) return;
  loading.value = true;
  try {
    const s = await api.lanSyncPickIp(ip);
    await applyState(s);
  } catch (e: any) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

async function copyText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    message.success(t("settings.lanSync.copied"));
  } catch {
    /* ignore */
  }
}

async function checkStatus() {
  try {
    const s = await api.lanSyncStatus();
    if (s.running) {
      running.value = true;
      await applyState(s);
    }
  } catch {
    /* ignore */
  }
}

checkStatus();
onMounted(async () => {
  unlisten = await listen<Transfer>("lan-sync://transfer", (e) => onTransfer(e.payload));
  staleTimer = window.setInterval(() => {
    // 手机中途断开不会有 done/error 事件：60 秒没动静就把卡片撤掉
    if (active.value && lastEventAt && Date.now() - lastEventAt > 60_000) {
      active.value = null;
      speedBps.value = 0;
    }
  }, 5000);
});
onUnmounted(() => {
  unlisten?.();
  unlisten = null;
  if (staleTimer) window.clearInterval(staleTimer);
});
</script>

<template>
  <div class="ls">
    <!-- 头部 -->
    <div class="ls-head">
      <div class="ls-title-row">
        <h3 class="ls-title">{{ t("settings.lanSync.title") }}</h3>
        <span class="ls-badge" :class="running ? 'on' : 'off'">
          <i class="ls-dot"></i>
          {{ running ? t("settings.lanSync.running") : t("settings.lanSync.idle") }}
        </span>
      </div>
      <button
        v-if="!running"
        class="ls-btn primary"
        :disabled="loading"
        @click="start"
      >
        {{ loading ? "…" : t("settings.lanSync.start") }}
      </button>
      <button
        v-else
        class="ls-btn danger"
        :disabled="loading"
        @click="stop"
      >
        {{ t("settings.lanSync.stop") }}
      </button>
    </div>

    <p v-if="error" class="ls-error">{{ error }}</p>

    <!-- 未启动 -->
    <template v-if="!running">
      <p class="ls-desc">{{ t("settings.lanSync.desc") }}</p>
    </template>

    <!-- 运行中 -->
    <template v-else-if="state">
      <div class="ls-active">
        <!-- 二维码 -->
        <div class="ls-qr-col">
          <div class="ls-qr-box">
            <img v-if="qrDataUrl" :src="qrDataUrl" alt="QR" class="ls-qr-img" />
          </div>
          <span class="ls-qr-cap">{{ t("settings.lanSync.scanTip") }}</span>
        </div>

        <!-- 信息 -->
        <div class="ls-info-col">
          <!-- 地址 -->
          <div class="ls-addr-block">
            <span class="ls-label">{{ t("settings.lanSync.serverAddr") }}</span>
            <div class="ls-addr-row">
              <code class="ls-code">{{ state.ip }}:{{ state.port }}</code>
              <button class="ls-link" @click="copyText(`${state.ip}:${state.port}`)">
                {{ t("settings.lanSync.copyAddr") }}
              </button>
              <button class="ls-link" @click="copyText(state.qrUrl)">
                {{ t("settings.lanSync.copyLink") }}
              </button>
            </div>
          </div>

          <!-- 多网卡切换 -->
          <div v-if="multiAdapter" class="ls-adapter-block">
            <span class="ls-label">{{ t("settings.lanSync.pickAdapter") }}</span>
            <div class="ls-ip-chips">
              <button
                v-for="candidate in state.ips"
                :key="candidate"
                class="ls-ip-chip"
                :class="{ active: candidate === state.ip }"
                @click="pickIp(candidate)"
              >
                {{ candidate }}
              </button>
            </div>
            <p class="ls-adapter-hint">{{ t("settings.lanSync.multiAdapterHint") }}</p>
          </div>

          <!-- 步骤 -->
          <div class="ls-steps">
            <div class="ls-step"><span class="ls-step-n">1</span><span>{{ t("settings.lanSync.step1") }}</span></div>
            <div class="ls-step"><span class="ls-step-n">2</span><span>{{ t("settings.lanSync.step2") }}</span></div>
            <div class="ls-step"><span class="ls-step-n">3</span><span>{{ t("settings.lanSync.step3") }}</span></div>
          </div>

          <p class="ls-security">ⓘ {{ t("settings.lanSync.securityHint") }}</p>
        </div>
      </div>

      <!-- ── 传输状态 ─────────────────────────────────────────── -->
      <div class="ls-transfer">
        <div class="ls-transfer-head">
          <span class="ls-label">{{ t("settings.lanSync.transferTitle") }}</span>
          <span v-if="active" class="ls-transfer-dir">
            {{ active.direction === "download"
              ? t("settings.lanSync.dirDownload")
              : t("settings.lanSync.dirUpload") }}
          </span>
        </div>

        <!-- 进行中 -->
        <div v-if="active" class="ls-tr-card">
          <div class="ls-tr-row">
            <strong class="ls-tr-name">{{ active.instanceName }}</strong>
            <span class="ls-tr-phase">{{ phaseText(active) }}</span>
          </div>

          <div class="ls-bar">
            <div class="ls-bar-fill" :style="{ width: activePercent + '%' }"></div>
          </div>

          <div class="ls-tr-meta">
            <span>{{ humanBytes(active.bytesDone) }} / {{ humanBytes(active.bytesTotal) }}</span>
            <span>{{ activePercent }}%</span>
          </div>

          <div class="ls-tr-sub">
            <span v-if="speedBps > 0">{{ t("settings.lanSync.speedLabel") }} {{ humanBytes(speedBps) }}/s</span>
            <span v-if="activeEta">{{ t("settings.lanSync.leftLabel") }} {{ activeEta }}</span>
          </div>

          <p v-if="active.phase === 'pack' && active.file" class="ls-tr-file">
            {{ t("settings.lanSync.fileLabel") }} · {{ active.file }}
          </p>
          <p v-if="active.phase === 'error' && active.message" class="ls-error">{{ active.message }}</p>
        </div>

        <!-- 空闲 -->
        <p v-else class="ls-tr-idle">{{ t("settings.lanSync.transferIdle") }}</p>

        <!-- 最近 -->
        <div v-if="recent.length" class="ls-tr-recent">
          <div class="ls-tr-recent-head">
            <span class="ls-label">{{ t("settings.lanSync.transferRecent") }}</span>
            <button class="ls-link" @click="recent = []">
              {{ t("settings.lanSync.transferClear") }}
            </button>
          </div>
          <div v-for="r in recent" :key="r.id" class="ls-tr-item">
            <span class="ls-tr-ok" :class="r.phase === 'error' ? 'bad' : 'good'">
              {{ r.phase === "error" ? "✕" : "✓" }}
            </span>
            <span class="ls-tr-item-name">{{ r.instanceName }}</span>
            <span class="ls-tr-item-size">{{ humanBytes(r.bytesTotal || r.bytesDone) }}</span>
            <span v-if="r.phase === 'error' && r.message" class="ls-tr-item-err">{{ r.message }}</span>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.ls {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

/* 头部 */
.ls-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.ls-title-row {
  display: flex;
  align-items: center;
  gap: 10px;
}
.ls-title {
  margin: 0;
  font-size: 15px;
  font-weight: 600;
  color: var(--text-1);
}
.ls-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  padding: 2px 9px;
  border-radius: 20px;
  font-weight: 500;
}
.ls-badge.on {
  color: var(--success);
  background: var(--success-12);
}
.ls-badge.off {
  color: var(--text-3);
  background: var(--w-06);
}
.ls-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: currentColor;
}
.ls-badge.on .ls-dot {
  box-shadow: 0 0 5px currentColor;
}

/* 按钮 */
.ls-btn {
  padding: 6px 16px;
  font-size: 12px;
  font-weight: 600;
  border-radius: 8px;
  cursor: pointer;
  border: 1px solid transparent;
  transition: all 0.12s;
  font-family: inherit;
}
.ls-btn.primary {
  background: var(--accent);
  color: var(--bg-0);
}
.ls-btn.primary:hover {
  background: var(--accent-deep);
}
.ls-btn.danger {
  background: var(--danger-12);
  border-color: var(--danger-30);
  color: var(--danger-text);
}
.ls-btn.danger:hover {
  background: var(--danger-16);
}
.ls-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.ls-error {
  margin: 0;
  padding: 8px 12px;
  border-radius: 8px;
  background: var(--danger-12);
  border: 1px solid var(--danger-30);
  color: var(--danger-text);
  font-size: 12px;
}

/* 未启动 */
.ls-desc {
  margin: 0;
  font-size: 13px;
  line-height: 1.6;
  color: var(--text-2);
}
/* 运行中 */
.ls-active {
  display: flex;
  gap: 22px;
  align-items: flex-start;
  flex-wrap: wrap;
}

/* 二维码列 */
.ls-qr-col {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.ls-qr-box {
  padding: 10px;
  background: #fff;
  border-radius: 14px;
  box-shadow: 0 0 0 1px var(--border), 0 8px 24px var(--k-30);
}
.ls-qr-img {
  width: 140px;
  height: 140px;
  display: block;
}
.ls-qr-cap {
  font-size: 11px;
  color: var(--text-3);
  text-align: center;
  max-width: 160px;
}

/* 信息列 */
.ls-info-col {
  flex: 1;
  min-width: 200px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.ls-label {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  display: block;
  margin-bottom: 6px;
}
.ls-addr-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}
.ls-code {
  font-family: var(--mono, "Cascadia Code", Consolas, monospace);
  font-size: 13px;
  padding: 4px 10px;
  border-radius: 8px;
  background: var(--w-08);
  color: var(--text-1);
  border: 1px solid var(--border);
}
.ls-link {
  background: transparent;
  border: none;
  color: var(--accent);
  font-size: 12px;
  cursor: pointer;
  padding: 2px 4px;
  border-radius: 4px;
  font-family: inherit;
}
.ls-link:hover {
  text-decoration: underline;
}

/* 网卡切换 */
.ls-adapter-block {
  margin-top: -4px;
}
.ls-ip-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.ls-ip-chip {
  font-family: var(--mono, "Cascadia Code", Consolas, monospace);
  font-size: 11px;
  padding: 3px 9px;
  border-radius: 6px;
  border: 1px solid var(--border);
  background: var(--w-04);
  color: var(--text-2);
  cursor: pointer;
  transition: all 0.12s;
}
.ls-ip-chip:hover {
  border-color: var(--accent-35);
}
.ls-ip-chip.active {
  background: var(--accent-soft);
  border-color: var(--accent);
  color: var(--accent);
}
.ls-adapter-hint {
  margin: 6px 0 0;
  font-size: 11px;
  color: var(--text-3);
}

/* 步骤 */
.ls-steps {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 14px;
  border: 1px solid var(--border);
  border-radius: 12px;
  background: var(--panel);
}
.ls-step {
  display: flex;
  align-items: center;
  gap: 10px;
}
.ls-step-n {
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: var(--w-08);
  color: var(--text-2);
  font-size: 10px;
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.ls-step > span:last-child {
  font-size: 12px;
  color: var(--text-2);
}

.ls-security {
  margin: 0;
  font-size: 11px;
  color: var(--text-3);
  line-height: 1.5;
}

/* ── 传输状态 ───────────────────────────────────────────────── */
.ls-transfer {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-top: 12px;
  border-top: 1px solid var(--border);
}
.ls-transfer-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.ls-transfer-dir {
  font-size: 11px;
  color: var(--accent);
  background: var(--w-08);
  padding: 1px 8px;
  border-radius: 999px;
}
.ls-tr-card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px;
  border-radius: 10px;
  background: var(--panel);
  border: 1px solid var(--border);
}
.ls-tr-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.ls-tr-name {
  font-size: 13px;
  color: var(--text-0);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ls-tr-phase {
  font-size: 11px;
  color: var(--accent);
  flex-shrink: 0;
}
.ls-bar {
  height: 6px;
  border-radius: 999px;
  background: var(--w-08);
  overflow: hidden;
}
.ls-bar-fill {
  height: 100%;
  border-radius: 999px;
  background: linear-gradient(90deg, var(--accent), var(--success));
  transition: width 0.25s ease;
}
.ls-tr-meta,
.ls-tr-sub {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 11px;
  color: var(--text-2);
}
.ls-tr-file {
  margin: 0;
  font-size: 11px;
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ls-tr-idle {
  margin: 0;
  font-size: 12px;
  color: var(--text-3);
}
.ls-tr-recent {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-top: 4px;
}
.ls-tr-recent-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.ls-tr-item {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--text-2);
}
.ls-tr-ok.good {
  color: var(--success);
}
.ls-tr-ok.bad {
  color: var(--danger);
}
.ls-tr-item-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ls-tr-item-size {
  flex-shrink: 0;
  color: var(--text-3);
}
.ls-tr-item-err {
  color: var(--danger);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
