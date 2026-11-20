<script setup lang="ts">
import { createLatestRequest } from "../composables/latestRequest";
import { motionScrollBehavior } from "../composables/useMotionPreference";
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { NButton, NInput, NSelect, NSpin, useDialog, useMessage } from "naive-ui";
import { api } from "../api";
import { fmtDateLocale as fmtTime, fmtSize } from "../utils/format";
import { error as devError } from "../utils/logger";
import type { CrashAIAnalysis, CrashDiagnosis } from "../types";
import {
  IconAlertCircle,
  IconBug,
  IconChevronDown,
  IconChevronRight,
  IconCopy,
  IconFile,
  IconRefresh,
  IconTrash,
} from "./icons";
import { useI18n } from "vue-i18n";

const props = defineProps<{ instanceId: string }>();
const message = useMessage();
const dialog = useDialog();
const { t } = useI18n();

const loading = ref(false);
const analyzing = ref(false);
const logs = ref<{ filename: string; modified: number; size: number; kind: string }[]>([]);
const selected = ref<string>("");
const diagnosis = ref<CrashDiagnosis | null>(null);
const rawContent = ref("");
const showRaw = ref(false);
const aiLoading = ref(false);
const aiResult = ref<CrashAIAnalysis | null>(null);
const aiError = ref("");
const aiConsentOpen = ref(false);
const aiPanelRef = ref<HTMLElement | null>(null);
const aiFeedbackOpen = ref(false);
const aiFeedbackReason = ref<string | null>(null);
const aiFeedbackDetail = ref("");
const aiFeedbackSent = ref(false);
const aiFeedbackSending = ref(false);
const aiFeedbackReasonKeys = ["wrong", "repeat", "generic", "no_evidence", "verbose", "other"];
const aiFeedbackReasonOptions = computed(() =>
  aiFeedbackReasonKeys.map((r) => ({
    label: t(`crashAnalyzer.aiFeedback.${r}`),
    value: r,
  })),
);

async function scrollToAIPanel(align: "start" | "end" = "end") {
  await nextTick();
  const apply = () => {
    const el = aiPanelRef.value;
    if (!el) return;
    // 只滚 app 的内容区，不用 scrollIntoView：后者在极端布局下会滚动
    // 整个文档，把普通 flex 布局的标题栏顶出视口。
    let scroller: HTMLElement | null = el.parentElement;
    while (scroller) {
      const overflowY = getComputedStyle(scroller).overflowY;
      if ((overflowY === "auto" || overflowY === "scroll") && scroller.scrollHeight > scroller.clientHeight) break;
      if (scroller.id === "app-content") break;
      scroller = scroller.parentElement;
    }
    if (!scroller) scroller = document.getElementById("app-content") as HTMLElement | null;
    if (!scroller) return;
    if (align === "end") {
      scroller.scrollTo({ top: scroller.scrollHeight, behavior: motionScrollBehavior() });
    } else {
      const top = el.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop - 12;
      scroller.scrollTo({ top: Math.max(0, top), behavior: motionScrollBehavior() });
    }
  };
  requestAnimationFrame(apply);
  // 展开动画/字体排版会改变高度，动画结束后再补一次，避免只露卡片头
  setTimeout(apply, 260);
}

// Tauri 的 invoke 报错可能是字符串，也可能是带 message 的对象，
// 统一转成可读文本，避免界面上只弹出一个 [object Object] 或啥都没有。
function errText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
}

// —— 分析结果缓存 ——
// 崩溃报告内容基本不会变，每次切走再回来都重新分析既慢又费（要读文件 + 跑诊断）。
// 按 实例 + 文件名 缓存诊断，TTL 1 小时。重新分析会强制刷新。
const DIAG_TTL = 7 * 24 * 60 * 60 * 1000;
interface DiagCache {
  d: CrashDiagnosis;
  ts: number;
}
function diagKey(instanceId: string, filename: string) {
  return `qookix:crash_diag:${instanceId}:${filename}`;
}
function readDiagCache(instanceId: string, filename: string): CrashDiagnosis | null {
  try {
    const raw = localStorage.getItem(diagKey(instanceId, filename));
    if (!raw) return null;
    const c = JSON.parse(raw) as DiagCache;
    if (Date.now() - c.ts > DIAG_TTL) return null;
    return normalize(c.d);
  } catch {
    return null;
  }
}
function writeDiagCache(instanceId: string, filename: string, d: CrashDiagnosis) {
  try {
    localStorage.setItem(diagKey(instanceId, filename), JSON.stringify({ d, ts: Date.now() }));
  } catch {
    /* 容量溢出等忽略，不影响主流程 */
  }
}

const logsRequest = createLatestRequest();
const analysisRequest = createLatestRequest();
const rawRequest = createLatestRequest();
const aiRequest = createLatestRequest();
const feedbackRequest = createLatestRequest();

function reportScope(request: ReturnType<typeof createLatestRequest>) {
  const ownsRequest = request.start();
  const instanceId = props.instanceId;
  const filename = selected.value;
  return {
    instanceId,
    filename,
    isCurrent: () => ownsRequest() && instanceId === props.instanceId && filename === selected.value,
  };
}

function invalidateReport() {
  analysisRequest.invalidate();
  rawRequest.invalidate();
  aiRequest.invalidate();
  feedbackRequest.invalidate();
  analyzing.value = false;
  aiLoading.value = false;
  aiFeedbackSending.value = false;
  aiFeedbackOpen.value = false;
  aiFeedbackSent.value = false;
  diagnosis.value = null;
  rawContent.value = "";
  showRaw.value = false;
  aiResult.value = null;
  aiError.value = "";
  aiConsentOpen.value = false;
}

onBeforeUnmount(() => {
  logsRequest.invalidate();
  invalidateReport();
});

async function loadLogs() {
  const ownsRequest = logsRequest.start();
  const instanceId = props.instanceId;
  const isCurrent = () => ownsRequest() && props.instanceId === instanceId;
  loading.value = true;
  try {
    const result = await api.crashAnalysis(instanceId);
    if (!isCurrent()) return;
    logs.value = result;
    if (logs.value.length && !selected.value) {
      await handleSelect(logs.value[0].filename);
    }
  } catch (error) {
    if (!isCurrent()) return;
    devError("[CrashAnalyzer] loadLogs failed:", error);
    message.error(t("crashAnalyzer.loadFailed", { error: errText(error) }));
  } finally {
    if (isCurrent()) loading.value = false;
  }
}

async function analyze(force = false) {
  if (!selected.value) {
    // 没选文件时给明确提示，而不是静默 return（否则看起来像"点了没反应"）
    if (!logs.value.length) {
      message.warning(t("crashAnalyzer.noReports"));
    } else {
      message.warning(t("crashAnalyzer.selectFirst"));
    }
    return;
  }
  const request = reportScope(analysisRequest);
  // 命中有效缓存且非强制刷新：直接展示，不发请求
  if (!force) {
    const cached = readDiagCache(request.instanceId, request.filename);
    if (cached) {
      diagnosis.value = cached;
      return;
    }
  }
  analyzing.value = true;
  diagnosis.value = null;
  rawContent.value = "";
  aiResult.value = null;
  aiError.value = "";
  try {
    const result = normalize(await api.analyzeCrash(request.instanceId, request.filename));
    if (!request.isCurrent()) return;
    diagnosis.value = result;
    writeDiagCache(request.instanceId, request.filename, result);
  } catch (e) {
    if (!request.isCurrent()) return;
    devError("[CrashAnalyzer] analyze failed:", e);
    message.error(t("crashAnalyzer.analyzeFailed", { error: errText(e) }));
  } finally {
    if (request.isCurrent()) analyzing.value = false;
  }
}

async function loadRaw() {
  if (!selected.value) {
    message.warning(t("crashAnalyzer.selectFirst"));
    return;
  }
  const request = reportScope(rawRequest);
  try {
    const content = await api.getCrashReportContent(request.instanceId, request.filename);
    if (request.isCurrent()) rawContent.value = content;
  } catch (e) {
    if (!request.isCurrent()) return;
    devError("[CrashAnalyzer] loadRaw failed:", e);
    message.error(t("crashAnalyzer.readFailed", { error: errText(e) }));
  }
}

// 点「查看原始报告」时：若还没加载过原始内容就顺手拉取，不必再单独点一次
async function ensureRaw() {
  if (rawContent.value) return;
  await loadRaw();
}

// 展开/收起原始报告：展开时若未加载则自动拉取
async function toggleRaw() {
  showRaw.value = !showRaw.value;
  if (showRaw.value) await ensureRaw();
}

function deleteLog(filename: string) {
  const instanceId = props.instanceId;
  dialog.warning({
    title: t("crashAnalyzer.delete"),
    content: t("crashAnalyzer.confirmDelete", { filename }),
    positiveText: t("common.delete"),
    negativeText: t("common.cancel"),
    onPositiveClick: async () => {
      try {
        await api.deleteInstancePath(instanceId, `crash-reports/${filename}`);
        localStorage.removeItem(diagKey(instanceId, filename));
        if (props.instanceId !== instanceId) return;
        if (selected.value === filename) {
          invalidateReport();
          selected.value = "";
        }
        message.success(t("crashAnalyzer.deleted"));
        await loadLogs();
      } catch (e) {
        message.error(String(e));
      }
    },
  });
}

function copyExcerpt() {
  if (!diagnosis.value?.excerpt) return;
  try {
    navigator.clipboard.writeText(diagnosis.value.excerpt);
    message.success(t("crashAnalyzer.copied"));
  } catch {
    message.error(t("crashAnalyzer.copyFailed"));
  }
}

function copyRaw() {
  if (!rawContent.value) return;
  try {
    navigator.clipboard.writeText(rawContent.value);
    message.success(t("crashAnalyzer.copied"));
  } catch {
    message.error(t("crashAnalyzer.copyFailed"));
  }
}

function copyStack() {
  if (!diagnosis.value?.stacktrace.length) return;
  try {
    navigator.clipboard.writeText(diagnosis.value.stacktrace.join("\n"));
    message.success(t("crashAnalyzer.copied"));
  } catch {
    message.error(t("crashAnalyzer.copyFailed"));
  }
}

/** 主因已在上方单独展示，这里列出其余命中原因 */
const otherCauses = computed(() => (diagnosis.value?.causes ?? []).slice(1));

/**
 * 兼容旧缓存：7 天 TTL 内可能存着没有 causes/stacktrace/details 的旧结构，
 * 直接读字段会在模板里炸，这里统一补齐默认值。
 */
function normalize(d: CrashDiagnosis): CrashDiagnosis {
  return {
    ...d,
    causes: d.causes ?? [],
    stacktrace: d.stacktrace ?? [],
    details: d.details ?? [],
    affected_mods: d.affected_mods ?? [],
    confidence: typeof d.confidence === "number" ? d.confidence : 0,
  };
}

// 严重度 → 中文名 / 前景色 / 背景色。主因与分因共用同一套配色。
function severityLabel(s: string): string {
  switch (s) {
    case "oom":
      return t("crashAnalyzer.severity.oom");
    case "jvm":
      return t("crashAnalyzer.severity.jvm");
    case "gl":
      return t("crashAnalyzer.severity.gl");
    case "mod":
      return t("crashAnalyzer.severity.mod");
    case "lwjgl":
      return t("crashAnalyzer.severity.lwjgl");
    case "java_ver":
      return t("crashAnalyzer.severity.javaVer");
    default:
      return t("crashAnalyzer.severity.unknown");
  }
}

function severityColor(s: string): string {
  switch (s) {
    case "oom":
      return "#e5534b";
    case "jvm":
      return "#e0a030";
    case "gl":
      return "#5aa2f0";
    case "mod":
      return "#7ad08a";
    case "lwjgl":
      return "#c78aff";
    case "java_ver":
      return "#ff7a90";
    default:
      return "#8b8e9c";
  }
}

function severityBg(s: string): string {
  switch (s) {
    case "oom":
      return "rgba(229,83,75,0.12)";
    case "jvm":
      return "rgba(224,160,48,0.12)";
    case "gl":
      return "rgba(90,162,240,0.12)";
    case "mod":
      return "rgba(122,208,138,0.12)";
    case "lwjgl":
      return "rgba(199,138,255,0.12)";
    case "java_ver":
      return "rgba(255,122,144,0.12)";
    default:
      return "rgba(139,142,156,0.12)";
  }
}

/** 置信度 → 文案 */
function confidenceLabel(c: number): string {
  if (c >= 85) return t("crashAnalyzer.confidence.high");
  if (c >= 60) return t("crashAnalyzer.confidence.medium");
  if (c > 0) return t("crashAnalyzer.confidence.low");
  return t("crashAnalyzer.confidence.none");
}

watch(
  () => props.instanceId,
  () => {
    invalidateReport();
    logs.value = [];
    selected.value = "";
    void loadLogs();
  },
  { immediate: true }
);

// 注意：不要用 watch(selected) 清空 diagnosis——它会在 handleSelect 之后执行，
// 把刚恢复的缓存又清掉，表现为「切走再切回来必须手动点分析」。
async function handleSelect(filename: string) {
  invalidateReport();
  selected.value = filename;
  const cached = readDiagCache(props.instanceId, filename);
  diagnosis.value = cached;
  // 没有缓存就自动分析，省去手动点「分析此崩溃报告」
  if (!cached) await analyze();
}

function startAIAnalyze() {
  if (!diagnosis.value || aiLoading.value) return;
  // 只打开应用内确认，绝不在这里发请求。
  // 旧结果先清掉，避免确认卡片背后还挂着上一次的回答。
  aiResult.value = null;
  aiError.value = "";
  aiConsentOpen.value = true;
  void scrollToAIPanel("end");
}

function cancelAIConsent() {
  aiConsentOpen.value = false;
}

async function agreeAIAnalyze() {
  aiConsentOpen.value = false;
  await runAIAnalyze(true);
}

// consented 必须由「同意并分析」按钮显式传入 true；
// 这是唯一的 AI 请求入口，任何默认调用都不会发送数据。
async function runAIAnalyze(consented = false) {
  if (!consented) {
    startAIAnalyze();
    return;
  }
  if (!diagnosis.value || !selected.value) return;
  const request = reportScope(aiRequest);
  aiLoading.value = true;
  aiError.value = "";
  aiResult.value = null;
  aiFeedbackOpen.value = false;
  aiFeedbackReason.value = null;
  aiFeedbackDetail.value = "";
  aiFeedbackSent.value = false;
  void scrollToAIPanel("end");
  try {
    const result = await api.aiAnalyzeCrash(request.instanceId, request.filename);
    if (request.isCurrent()) aiResult.value = result;
  } catch (e) {
    if (!request.isCurrent()) return;
    devError("[CrashAnalyzer] AI analyze failed:", e);
    aiError.value = errText(e);
    message.error(t("crashAnalyzer.aiFailed", { error: aiError.value }));
  } finally {
    if (request.isCurrent()) {
      aiLoading.value = false;
      void scrollToAIPanel(aiResult.value ? "start" : "end");
    }
  }
}

async function sendAIFeedback(helpful: boolean) {
  if (!helpful) {
    aiFeedbackOpen.value = true;
    return;
  }
  await submitAIFeedback(true, "", "");
}

async function sendAINegativeFeedback() {
  if (!aiFeedbackReason.value) {
    message.warning(t("crashAnalyzer.aiFeedbackNeedReason"));
    return;
  }
  await submitAIFeedback(false, aiFeedbackReason.value || "", aiFeedbackDetail.value);
}

async function submitAIFeedback(helpful: boolean, reason: string, detail: string) {
  if (!aiResult.value?.hash || aiFeedbackSending.value || aiFeedbackSent.value) return;
  const request = reportScope(feedbackRequest);
  aiFeedbackSending.value = true;
  try {
    await api.submitCrashAIFeedback(aiResult.value.hash, helpful, reason, detail);
    if (!request.isCurrent()) return;
    aiFeedbackSent.value = true;
    aiFeedbackOpen.value = false;
  } catch (e) {
    if (!request.isCurrent()) return;
    devError("[CrashAnalyzer] AI feedback failed:", e);
    message.error(t("crashAnalyzer.aiFeedbackFailed", { error: errText(e) }));
  } finally {
    if (request.isCurrent()) aiFeedbackSending.value = false;
  }
}
</script>

<template>
  <div class="crash-analyzer">
    <div v-if="loading" class="crash-loading">
      <NSpin size="medium" />
      <span>{{ t("crashAnalyzer.scanning") }}</span>
    </div>

    <div v-else-if="logs.length === 0" class="crash-empty">
      <IconBug />
      <p>{{ t("crashAnalyzer.empty") }}</p>
      <span class="crash-empty-hint">{{ t("crashAnalyzer.emptyHint") }}</span>
    </div>

    <div v-else class="crash-body">
      <!-- 日志列表 -->
      <div class="crash-log-list">
        <div class="crash-log-header">
          <span class="crash-log-title">{{ t("crashAnalyzer.listTitle") }}</span>
          <span class="crash-log-count">{{ t("crashAnalyzer.count", { n: logs.length }) }}</span>
        </div>
        <div class="crash-log-items">
          <div
            v-for="l in logs"
            :key="l.filename"
            class="crash-log-item"
            :class="{ active: selected === l.filename }"
          >
            <button class="crash-log-select" @click="handleSelect(l.filename)">
              <div class="crash-log-icon">
                <IconFile v-if="l.kind === 'crash'" />
                <IconAlertCircle v-else />
              </div>
              <div class="crash-log-info">
                <div class="crash-log-name text-ellipsis">{{ l.filename }}</div>
                <div class="crash-log-meta">
                  <span>{{ l.kind === "crash" ? t("crashAnalyzer.kindCrash") : t("crashAnalyzer.kindJvm") }}</span>
                  <span>·</span>
                  <span>{{ fmtSize(l.size) }}</span>
                  <span>·</span>
                  <span>{{ fmtTime(l.modified) }}</span>
                </div>
              </div>
            </button>
            <button
              class="crash-log-del"
              :title="t('crashAnalyzer.delete')"
              @click="deleteLog(l.filename)"
            >
              <IconTrash />
            </button>
          </div>
        </div>
      </div>

      <!-- 分析结果 -->
      <div class="crash-result">
        <div v-if="analyzing" class="crash-analyzing">
          <NSpin size="medium" />
          <span>{{ t("crashAnalyzer.analyzing") }}</span>
        </div>

        <!-- 已出诊断：优先于「分析」按钮展示，否则点完按钮结果永远不显示 -->
        <div v-else-if="diagnosis" class="crash-diagnosis">
          <!-- 结论摘要 -->
          <header class="crash-summary">
            <div class="crash-summary-top">
              <div
                class="crash-severity"
                :style="{ background: severityBg(diagnosis.severity), color: severityColor(diagnosis.severity) }"
              >
                <span class="crash-severity-dot" :style="{ background: severityColor(diagnosis.severity) }" />
                <span class="crash-severity-text">{{ severityLabel(diagnosis.severity) }}</span>
              </div>
              <div class="crash-confidence">
                <span>{{ confidenceLabel(diagnosis.confidence) }} · {{ diagnosis.confidence }}%</span>
                <div class="crash-confidence-track">
                  <i
                    :style="{
                      width: Math.max(4, diagnosis.confidence) + '%',
                      background: severityColor(diagnosis.severity),
                    }"
                  />
                </div>
              </div>
            </div>
            <h3 class="crash-title">{{ diagnosis.title }}</h3>
            <p class="crash-reason">{{ diagnosis.reason }}</p>
          </header>

          <!-- 修复建议：先给可执行动作 -->
          <div class="crash-section">
            <div class="crash-section-label">{{ t("crashAnalyzer.sectionAdvice") }}</div>
            <p class="crash-advice">{{ diagnosis.advice }}</p>
          </div>

          <!-- 关键堆栈 -->
          <div v-if="diagnosis.stacktrace.length" class="crash-section">
            <div class="crash-section-label">
              {{ t("crashAnalyzer.sectionStack") }}
              <NButton quaternary size="tiny" @click="copyStack">
                <IconCopy />
              </NButton>
            </div>
            <div class="crash-stack">
              <div v-for="(f, i) in diagnosis.stacktrace" :key="i" class="crash-stack-line">
                {{ f }}
              </div>
            </div>
          </div>

          <!-- 运行环境 -->
          <div v-if="diagnosis.details.length" class="crash-section">
            <div class="crash-section-label">{{ t("crashAnalyzer.sectionEnv") }}</div>
            <div class="crash-details">
              <div v-for="d in diagnosis.details" :key="d.key" class="crash-detail">
                <span class="crash-detail-key">{{ d.key }}</span>
                <span class="crash-detail-value text-ellipsis">{{ d.value }}</span>
              </div>
            </div>
          </div>

          <!-- 其他可能原因 -->
          <div v-if="otherCauses.length" class="crash-section">
            <div class="crash-section-label">{{ t("crashAnalyzer.sectionOtherCauses", { n: otherCauses.length }) }}</div>
            <div class="crash-other-causes">
              <div v-for="c in otherCauses" :key="c.id" class="crash-other-cause">
                <div class="crash-other-head">
                  <span
                    class="crash-other-tag"
                    :style="{ background: severityBg(c.severity), color: severityColor(c.severity) }"
                  >
                    {{ severityLabel(c.severity) }}
                  </span>
                  <span class="crash-other-title">{{ c.title }}</span>
                  <span class="crash-other-conf">{{ c.confidence }}%</span>
                </div>
                <p class="crash-other-advice">{{ c.advice }}</p>
              </div>
            </div>
          </div>

          <!-- 受影响模组 -->
          <div v-if="diagnosis.affected_mods.length" class="crash-section">
            <div class="crash-section-label">{{ t("crashAnalyzer.sectionAffectedMods") }}</div>
            <div class="crash-mods">
              <span v-for="m in diagnosis.affected_mods" :key="m" class="crash-mod">{{ m }}</span>
            </div>
          </div>

          <!-- 关键摘要 -->
          <div v-if="diagnosis.excerpt" class="crash-section">
            <div class="crash-section-label">
              {{ t("crashAnalyzer.sectionKeyInfo") }}
              <NButton quaternary size="tiny" @click="copyExcerpt">
                <IconCopy />
              </NButton>
            </div>
            <p class="crash-excerpt">{{ diagnosis.excerpt }}</p>
          </div>

          <!-- 操作 -->
          <div class="crash-actions">
            <NButton type="primary" size="small" :loading="aiLoading" @click="startAIAnalyze">
              {{ aiResult ? t("crashAnalyzer.aiReanalyze") : t("crashAnalyzer.aiAnalyze") }}
            </NButton>
            <NButton quaternary size="small" @click="analyze(true)">
              <IconRefresh />
              {{ t("crashAnalyzer.reanalyze") }}
            </NButton>
            <NButton quaternary size="small" @click="toggleRaw">
              <IconChevronRight v-if="!showRaw" />
              <IconChevronDown v-else />
              {{ t("crashAnalyzer.viewRaw") }}
            </NButton>
          </div>

          <!-- AI 深入分析：可选，本地诊断仍然优先；同意前绝不发请求 -->
          <div ref="aiPanelRef" class="crash-ai-panel">
          <Transition name="crash-ai-expand">
          <div v-if="aiConsentOpen" key="consent" class="crash-ai-box crash-ai-consent">
            <div class="crash-ai-head">
              <span class="crash-ai-badge">AI</span>
              <b>{{ t("crashAnalyzer.aiConsentTitle") }}</b>
            </div>
            <p class="crash-ai-consent-text">{{ t("crashAnalyzer.aiConfirmBody") }}</p>
            <p class="crash-ai-consent-text">{{ t("crashAnalyzer.aiConfirmPrivacy") }}</p>
            <div class="crash-ai-consent-actions">
              <NButton size="small" quaternary @click="cancelAIConsent">
                {{ t("crashAnalyzer.aiCancel") }}
              </NButton>
              <NButton size="small" type="primary" @click="agreeAIAnalyze">
                {{ t("crashAnalyzer.aiAgree") }}
              </NButton>
            </div>
          </div>
          <div v-else-if="aiLoading" key="loading" class="crash-ai-box crash-ai-loading">
            <NSpin size="small" />
            <span>{{ t("crashAnalyzer.aiAnalyzing") }}</span>
          </div>
          <div v-else-if="aiError" key="error" class="crash-ai-box crash-ai-error">
            {{ t("crashAnalyzer.aiFailed", { error: aiError }) }}
          </div>
          <div v-else-if="aiResult" key="result" class="crash-ai-box">
            <div class="crash-ai-head">
              <span class="crash-ai-badge">AI</span>
              <span class="crash-ai-meta">
                QookiX AI
                <template v-if="aiResult.cached"> · {{ t("crashAnalyzer.aiCached") }}</template>
              </span>
            </div>
            <p class="crash-ai-summary">{{ aiResult.result.summary }}</p>
            <div
              v-for="(c, i) in aiResult.result.likely_causes"
              :key="i"
              class="crash-ai-cause"
            >
              <div class="crash-ai-cause-head">
                <b>{{ c.title }}</b>
                <span>{{ c.confidence }}%</span>
              </div>
              <div v-if="c.next_steps.length" class="crash-ai-steps-wrap">
                <ul class="crash-ai-steps">
                  <li v-for="(s, j) in c.next_steps" :key="j">{{ s }}</li>
                </ul>
              </div>
              <p v-if="c.evidence" class="crash-ai-evidence">{{ c.evidence }}</p>
            </div>
            <div v-if="aiResult.result.need_more_info.length" class="crash-ai-need">
              <div class="crash-section-label">{{ t("crashAnalyzer.aiNeedMore") }}</div>
              <ul class="crash-ai-steps">
                <li v-for="(n, i) in aiResult.result.need_more_info" :key="i">{{ n }}</li>
              </ul>
            </div>
            <div v-if="aiResult.hash" class="crash-ai-feedback">
              <template v-if="!aiFeedbackSent">
                <span class="crash-ai-feedback-label">{{ t("crashAnalyzer.aiFeedbackAsk") }}</span>
                <NButton size="tiny" type="primary" :loading="aiFeedbackSending" @click="sendAIFeedback(true)">
                  {{ t("crashAnalyzer.aiFeedbackYes") }}
                </NButton>
                <NButton size="tiny" quaternary :disabled="aiFeedbackSending" @click="sendAIFeedback(false)">
                  {{ t("crashAnalyzer.aiFeedbackNo") }}
                </NButton>
                <div v-if="aiFeedbackOpen" class="crash-ai-feedback-form">
                  <NSelect
                    v-model:value="aiFeedbackReason"
                    :options="aiFeedbackReasonOptions"
                    :placeholder="t('crashAnalyzer.aiFeedbackChoose')"
                    size="tiny"
                    style="width: 100%;"
                  />
                  <NInput
                    v-model:value="aiFeedbackDetail"
                    type="textarea"
                    :rows="2"
                    :placeholder="t('crashAnalyzer.aiFeedbackDetail')"
                    size="tiny"
                  />
                  <div class="crash-ai-feedback-actions">
                    <NButton size="tiny" type="primary" :loading="aiFeedbackSending" @click="sendAINegativeFeedback">
                      {{ t("crashAnalyzer.aiFeedbackSubmit") }}
                    </NButton>
                    <NButton size="tiny" quaternary @click="aiFeedbackOpen = false">
                      {{ t("crashAnalyzer.aiCancel") }}
                    </NButton>
                  </div>
                </div>
              </template>
              <span v-else class="crash-ai-feedback-done">✓ {{ t("crashAnalyzer.aiFeedbackThanks") }}</span>
            </div>
            <p class="crash-ai-disclaimer">{{ aiResult.result.disclaimer || t("crashAnalyzer.aiDisclaimer") }}</p>
          </div>
          </Transition>
          </div>

          <!-- 原始内容：点「查看原始报告」即自动加载，无需再单独点一次 -->
          <div v-if="showRaw" class="crash-raw">
            <NSpin v-if="!rawContent" size="small" />
            <template v-else>
              <pre>{{ rawContent }}</pre>
              <NButton quaternary size="small" @click="copyRaw">
                <IconCopy />
                {{ t("crashAnalyzer.copyAll") }}
              </NButton>
            </template>
          </div>
        </div>

        <!-- 尚未分析：显示「分析此崩溃报告」按钮（selected 为真但还没出结果） -->
        <div v-else class="crash-prompt">
          <NButton type="primary" size="small" :disabled="analyzing || !selected" @click="analyze()">
            {{ t("crashAnalyzer.analyze") }}
          </NButton>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.crash-analyzer {
  display: flex;
  flex-direction: column;
  height: 100%;
  gap: 16px;
}

/* 磨砂玻璃适配：卡片与卡片式按钮统一加上背景模糊，跟随主题 --glass-blur */
.crash-result,
.crash-log-item,
.crash-excerpt,
.crash-raw,
.crash-advice {
  backdrop-filter: blur(var(--glass-blur, 8px));
  -webkit-backdrop-filter: blur(var(--glass-blur, 8px));
}

.crash-loading,
.crash-analyzing,
.crash-prompt {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 40px 0;
  color: var(--text-3);
  font-size: var(--fs-base);
}

.crash-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 60px 0;
  color: var(--text-3);
  font-size: var(--fs-base);
}
.crash-empty p {
  margin: 0;
  font-size: var(--fs-md);
  color: var(--text-2);
}
.crash-empty-hint {
  font-size: var(--fs-xs);
  color: var(--text-3);
}

.crash-body {
  display: grid;
  grid-template-columns: minmax(240px, 280px) minmax(0, 1fr);
  gap: 14px;
  height: 100%;
  min-height: 0;
}

/* 日志列表：一个分组面板，不再每项一张卡 */
.crash-log-list {
  display: flex;
  flex-direction: column;
  gap: 0;
  min-width: 0;
  padding: 10px;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: var(--r-md);
  backdrop-filter: blur(var(--glass-blur, 8px));
  -webkit-backdrop-filter: blur(var(--glass-blur, 8px));
}
.crash-log-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 2px 4px 10px;
}
.crash-log-title {
  font-size: var(--fs-sm);
  font-weight: 700;
  color: var(--text-1);
}
.crash-log-count {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  background: var(--panel);
  padding: 2px 8px;
  border-radius: var(--r-xl);
}
.crash-log-items {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: 1;
  overflow-y: auto;
  padding-right: 2px;
}
.crash-log-item {
  display: flex;
  align-items: center;
  gap: 10px;
  border-radius: var(--r-sm);
  border-left: 2px solid transparent;
  background: transparent;
  transition: background 0.16s ease, border-color 0.16s ease;
}
/* 选择区：撑满条目并承载 padding，使条目内除删除键外的区域都可点击 */
.crash-log-select {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border: none;
  background: transparent;
  color: inherit;
  font-family: inherit;
  text-align: left;
  cursor: pointer;
}
.crash-log-item:hover {
  background: var(--panel-hover);
}
.crash-log-item.active {
  border-left-color: var(--accent);
  background: var(--accent-soft);
}
.crash-log-icon {
  flex-shrink: 0;
  color: var(--text-3);
  font-size: 16px;
}
.crash-log-item.active .crash-log-icon {
  color: var(--accent);
}
.crash-log-info {
  flex: 1;
  min-width: 0;
}
.crash-log-name {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
}
.crash-log-meta {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  display: flex;
  gap: 4px;
  margin-top: 2px;
}
.crash-log-del {
  flex-shrink: 0;
  margin-right: 6px;
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  color: var(--text-3);
  border-radius: var(--r-xs);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  font-size: var(--fs-sm);
  opacity: 0;
  transition: all 0.12s;
}
.crash-log-item:hover .crash-log-del {
  opacity: 1;
}
.crash-log-del:hover {
  color: var(--danger);
  background: var(--danger-10);
}

/* 分析结果 */
/* 分析结果卡片 */
.crash-result {
  display: flex;
  flex-direction: column;
  gap: 18px;
  overflow-y: auto;
  min-width: 0;
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: var(--r-md);
  padding: 0 20px 28px;
}

.crash-summary {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0 -20px;
  padding: 16px 20px 18px;
  background: var(--panel-2, rgba(127, 127, 127, 0.05));
  border-bottom: 1px solid var(--border);
}
.crash-summary-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  flex-wrap: wrap;
}
.crash-severity {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 4px 10px;
  border-radius: var(--r-xl);
  border: 1px solid var(--border);
  font-size: var(--fs-xs);
  font-weight: 600;
  width: fit-content;
}
.crash-confidence {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-2xs);
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}
.crash-confidence-track {
  width: 96px;
  height: 4px;
  border-radius: 999px;
  background: var(--border);
  overflow: hidden;
}
.crash-confidence-track i {
  display: block;
  height: 100%;
  border-radius: inherit;
  transition: width 0.24s ease;
}

/* 其他可能原因 */
.crash-other-causes {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.crash-other-cause {
  border: 1px solid var(--border);
  border-left: 2px solid var(--border);
  border-radius: var(--r-sm);
  padding: 9px 12px;
  background: var(--panel);
  transition: border-left-color 0.16s ease;
}
.crash-other-cause:hover {
  border-left-color: var(--accent);
}
.crash-other-head {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.crash-other-tag {
  font-size: var(--fs-2xs);
  font-weight: 600;
  padding: 2px 8px;
  border-radius: var(--r-xl);
  flex-shrink: 0;
}
.crash-other-title {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
  flex: 1;
  min-width: 0;
}
.crash-other-conf {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  flex-shrink: 0;
}
.crash-other-advice {
  margin: 6px 0 0;
  font-size: var(--fs-xs);
  line-height: 1.6;
  color: var(--text-2);
}

/* 运行环境 */
.crash-details {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  border: 1px solid var(--border);
  border-radius: var(--r-sm);
  overflow: hidden;
  background: var(--panel);
}
.crash-detail {
  display: flex;
  gap: 10px;
  padding: 7px 12px;
  font-size: var(--fs-xs);
  border-bottom: 1px solid var(--border);
}
.crash-detail:nth-child(odd) {
  background: rgba(128, 128, 128, 0.035);
}
.crash-detail-key {
  flex-shrink: 0;
  width: 120px;
  color: var(--text-3);
}
.crash-detail-value {
  color: var(--text-1);
  min-width: 0;
}

/* 关键堆栈 */
.crash-stack {
  border: 1px solid var(--border);
  border-radius: var(--r-md);
  padding: 10px 12px;
  background: var(--panel);
  max-height: 220px;
  overflow: auto;
  font-family: "Cascadia Code", Consolas, monospace;
}
.crash-stack-line {
  font-size: var(--fs-xs);
  line-height: 1.6;
  color: var(--text-2);
  white-space: pre-wrap;
  word-break: break-all;
}
.crash-stack-line:first-child {
  color: var(--text-1);
  font-weight: 600;
}
.crash-severity-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
}

.crash-title {
  font-size: var(--fs-lg);
  font-weight: 700;
  color: var(--text-1);
  margin: 0;
  line-height: 1.3;
}

.crash-section {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.crash-section-label {
  font-size: var(--fs-2xs);
  font-weight: 700;
  color: var(--text-3);
  letter-spacing: 0.5px;
  display: flex;
  align-items: center;
  gap: 8px;
  text-transform: uppercase;
}
.crash-section-label::after {
  content: "";
  flex: 1;
  height: 1px;
  background: var(--border);
}
.crash-reason {
  margin: 0;
  font-size: var(--fs-base);
  color: var(--text-2);
  line-height: 1.6;
}
.crash-excerpt {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--text-1);
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: var(--r-sm);
  padding: 10px 12px;
  font-family: "Cascadia Code", Consolas, monospace;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}
.crash-mods {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.crash-mod {
  font-size: var(--fs-xs);
  padding: 3px 8px;
  border-radius: var(--r-xs);
  background: var(--accent-soft);
  color: var(--accent);
  font-weight: 600;
}
.crash-advice {
  margin: 0;
  font-size: var(--fs-base);
  color: var(--text-1);
  line-height: 1.7;
  background: var(--accent-soft);
  border: 1px solid var(--border);
  border-left: 3px solid var(--accent);
  border-radius: var(--r-sm);
  padding: 12px 14px;
}

.crash-actions {
  display: flex;
  gap: 10px;
  padding-top: 12px;
  margin-top: 4px;
  border-top: 1px solid var(--border);
  flex-wrap: wrap;
}

.crash-ai-panel {
  scroll-margin-top: 16px;
}

.crash-ai-box {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-top: 12px;
  padding: 16px;
  background: var(--panel-2, rgba(127, 127, 127, 0.06));
  border: 1px solid var(--border);
  border-left: 3px solid var(--accent);
  border-radius: var(--r-sm);
}

.crash-ai-expand-enter-active,
.crash-ai-expand-leave-active {
  transition: opacity 0.18s ease, transform 0.18s ease;
}
.crash-ai-expand-enter-from,
.crash-ai-expand-leave-to {
  opacity: 0;
  transform: translateY(-8px);
}

.crash-ai-head b {
  font-size: var(--fs-sm);
  color: var(--text-1);
}

.crash-ai-consent-text {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--text-2);
  line-height: 1.7;
}

.crash-ai-consent-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.crash-ai-loading {
  flex-direction: row;
  align-items: center;
  color: var(--text-3);
  font-size: var(--fs-sm);
}
.crash-ai-error {
  color: #e5534b;
  font-size: var(--fs-sm);
  border-color: rgba(229, 83, 75, 0.28);
}
.crash-ai-head {
  display: flex;
  align-items: center;
  gap: 8px;
}
.crash-ai-badge {
  font-size: var(--fs-2xs);
  font-weight: 800;
  letter-spacing: 0.08em;
  color: var(--accent);
  background: var(--accent-soft);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 2px 6px;
}
.crash-ai-meta {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.crash-ai-summary {
  margin: 0;
  font-size: var(--fs-base);
  color: var(--text-1);
  line-height: 1.7;
}
.crash-ai-cause {
  display: flex;
  flex-direction: column;
  gap: 5px;
  padding-top: 8px;
  border-top: 1px solid var(--border);
}
.crash-ai-cause-head {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  font-size: var(--fs-sm);
  color: var(--text-1);
}
.crash-ai-cause-head span {
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}
.crash-ai-steps-wrap {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.crash-ai-steps-wrap::before {
  content: "处理办法";
  font-size: var(--fs-2xs);
  font-weight: 700;
  color: var(--text-3);
}
.crash-ai-evidence {
  margin: 0;
  font-size: var(--fs-sm);
  color: var(--text-2);
  line-height: 1.6;
}
.crash-ai-evidence::before {
  content: "判断依据 · ";
  font-size: var(--fs-2xs);
  font-weight: 700;
  color: var(--text-3);
}
.crash-ai-steps {
  margin: 0;
  padding-left: 18px;
  font-size: var(--fs-sm);
  color: var(--text-2);
  line-height: 1.7;
}
.crash-ai-need {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.crash-ai-disclaimer {
  margin: 0;
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.crash-ai-feedback {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  padding-top: 8px;
  border-top: 1px solid var(--border);
}
.crash-ai-feedback-label {
  font-size: var(--fs-sm);
  color: var(--text-2);
  margin-right: 4px;
}
.crash-ai-feedback-form {
  flex-basis: 100%;
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 4px;
}
.crash-ai-feedback-actions {
  display: flex;
  gap: 6px;
}
.crash-ai-feedback-done {
  font-size: var(--fs-sm);
  color: var(--text-3);
}

.crash-raw {
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: var(--r-md);
  padding: 12px;
  overflow: auto;
  max-height: 300px;
}
.crash-raw pre {
  margin: 0;
  font-size: var(--fs-xs);
  font-family: "Cascadia Code", Consolas, monospace;
  line-height: 1.5;
  color: var(--text-2);
  white-space: pre-wrap;
  word-break: break-word;
}
</style>
