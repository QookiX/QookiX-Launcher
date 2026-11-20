<script setup lang="ts">
import { vSegmented } from "../../composables/vSegmented";
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { fmtSize } from "../../utils/format";
import { cacheClearPrefix } from "../../utils/cache";
import { NSelect, useMessage } from "naive-ui";
import { useSettingsStore } from "../../stores/settings";
import { api } from "../../api";
import { useSlidingIndicator } from "../../composables/useSlidingIndicator";
import type { MirrorPreset } from "../../types";
import { useI18n } from "vue-i18n";

const settings = useSettingsStore();
const { t } = useI18n();
const message = useMessage();

// 下载代理 seg 滑动高亮（系统代理 / 直连 / 自定义）
const proxyModeSegRef = ref<HTMLElement | null>(null);
// 文案跟着语言切换，所以用 computed（普通数组只在 setup 时算一次，切语言不会更新）
const proxyModes = computed(() => [
  { id: "system", label: t("settings.proxyMode.system") },
  { id: "direct", label: t("settings.proxyMode.direct") },
  { id: "custom", label: t("settings.proxyMode.custom") },
]);
const { indicatorStyle: proxyModeSegStyle, refresh: refreshProxyModeSeg } = useSlidingIndicator(
  proxyModeSegRef,
  () => Array.from(proxyModeSegRef.value?.querySelectorAll<HTMLElement>(".seg button") ?? []),
  () => Math.max(0, proxyModes.value.findIndex((m) => m.id === settings.settings?.proxy_mode)),
  { axis: "horizontal" }
);
watch(() => settings.settings?.proxy_mode, () => nextTick(() => refreshProxyModeSeg()));

async function selectProxyMode(id: string) {
  if (settings.settings?.proxy_mode === id) return;
  try {
    await settings.patch({ proxy_mode: id });
  } catch (e) {
    message.error(String(e));
  }
}

function onCustomProxyInput() {
  if (settings.settings && settings.settings.proxy_mode !== "custom") {
    settings.settings.proxy_mode = "custom";
  }
}

// 测试代理连接
const testingProxy = ref(false);
async function testProxy() {
  if (testingProxy.value || !settings.settings) return;
  testingProxy.value = true;
  try {
    const { proxy_mode, proxy } = settings.settings;
    // 自定义模式必须填地址，否则后端会退化为直连而误报成功
    if (proxy_mode === "custom" && !(proxy ?? "").trim()) {
      message.warning(t("settings.proxyTest.fillAddress"));
      return;
    }
    const res = await api.testProxy(
      proxy_mode,
      proxy_mode === "custom" ? proxy : null
    );
    message.success(t("settings.proxyTest.success", { ms: res.ms }));
  } catch (e) {
    message.error(t("settings.proxyTest.failed", { error: e }));
  } finally {
    testingProxy.value = false;
  }
}

const contentMirrors = ref<MirrorPreset[]>([]);
/** 每个内容镜像最近一次测速结果（毫秒）；null 表示不可用 */
const contentMirrorLatency = ref<Record<string, number | null>>({});
const testingContentMirror = ref("");

async function loadContentMirrors() {
  try {
    contentMirrors.value = await api.listContentMirrors();
  } catch {
    contentMirrors.value = [];
  }
}

async function selectContentMirror(id: string) {
  if (settings.settings?.content_mirror === id) return;
  try {
    await settings.patch({ content_mirror: id });
    // 发现页结果按来源缓存（内存 5 分钟 + localStorage 6 小时），换源后旧结果
    // 若继续命中，用户会以为镜像没生效；这里一并作废，回到发现页即重新请求。
    cacheClearPrefix("browse:");
    cacheClearPrefix("cf-cats:");
  } catch (e) {
    message.error(String(e));
  }
}

async function testContentMirror(id: string, base: string) {
  if (testingContentMirror.value) return;
  testingContentMirror.value = id;
  try {
    const res = await api.testContentMirror(base);
    contentMirrorLatency.value = { ...contentMirrorLatency.value, [id]: res.ms };
  } catch (e) {
    contentMirrorLatency.value = { ...contentMirrorLatency.value, [id]: null };
    message.error(String(e));
  } finally {
    testingContentMirror.value = "";
  }
}

function onCustomContentMirrorInput() {
  if (!settings.settings) return;
  if (settings.settings.content_mirror !== "custom") {
    settings.settings.content_mirror = "custom";
  }
  // 自定义地址改了，旧来源的缓存结果同样不能再命中
  cacheClearPrefix("browse:");
  cacheClearPrefix("cf-cats:");
}

// ---- 内容翻译 ----
const translateOptions = computed(() => [
  { label: t("settings.content.translateDefault"), value: "default" },
  { label: t("settings.content.translateCustom"), value: "custom" },
  { label: t("settings.content.translateBaidu"), value: "baidu_web" },
]);
const testingTranslate = ref(false);

// 两把 Key 都是「只写不读」：后端不下发 Key 原文（只给「配没配 + 尾号」），
// 所以输入框绑本地状态，填了才会写回后端，保存成功后清空。
const cfKey = ref("");
const trKey = ref("");
const savingKey = ref<"curseforge" | "translate" | "">("");

function keyField(which: "curseforge" | "translate") {
  return which === "curseforge" ? "curseforge_api_key" : "translate_api_key";
}

/** 保存输入框里的 Key（留空不动，避免把已存的 Key 冲掉） */
async function saveKey(which: "curseforge" | "translate") {
  const value = (which === "curseforge" ? cfKey : trKey).value.trim();
  if (!value) return;
  savingKey.value = which;
  try {
    await settings.patch({ [keyField(which)]: value });
    if (which === "curseforge") cfKey.value = "";
    else trKey.value = "";
    message.success(t("settings.content.keySaved"));
  } catch (e) {
    message.error(String(e));
  } finally {
    savingKey.value = "";
  }
}

/** 清掉已保存的 Key（空串 = 后端删除该 Key） */
async function clearKey(which: "curseforge" | "translate") {
  savingKey.value = which;
  try {
    await settings.patch({ [keyField(which)]: "" });
    message.success(t("settings.content.keyCleared"));
  } catch (e) {
    message.error(String(e));
  } finally {
    savingKey.value = "";
  }
}

async function testTranslate() {
  const s = settings.settings;
  if (!s) return;
  const key = trKey.value.trim();
  // Key 可以留空：表示用已保存的那把（后端会自己取）
  if (!s.translate_api_base || !s.translate_api_model || (!key && !s.translate_api_key_set)) {
    message.warning(t("settings.content.translateMissingFields"));
    return;
  }
  testingTranslate.value = true;
  try {
    await api.testTranslateApi(s.translate_api_base, key, s.translate_api_model);
    message.success(t("settings.content.translateOk"));
  } catch (e) {
    message.error(String(e));
  } finally {
    testingTranslate.value = false;
  }
}

// ---- 内容翻译缓存 ----
const clearingCacheService = ref<"default" | "custom" | null>(null);
async function clearTranslations(service: "default" | "custom") {
  if (clearingCacheService.value) return;
  clearingCacheService.value = service;
  const label = service === "custom" ? t("settings.content.customApiLabel") : t("settings.content.builtinLabel");
  try {
    const freed = await api.clearTranslationCache(service);
    message.success(
      freed > 0 ? t("settings.content.cacheClearedFreed", { label, size: fmtSize(freed) }) : t("settings.content.cacheCleared", { label })
    );
  } catch (e) {
    message.error(String(e));
  } finally {
    clearingCacheService.value = null;
  }
}

onMounted(loadContentMirrors);
</script>

<template>
  <div v-if="settings.settings" class="settings-body">
    <div class="settings-pane">
      <div class="grid">
        <div class="card glass" id="content-curseforge">
          <h3>{{ t("settings.content.curseforgeKey") }}</h3>
          <div class="proxy-row">
            <input
              v-model="cfKey"
              class="text-input mono"
              :placeholder="t('settings.content.curseforgePlaceholder')"
              @change="saveKey('curseforge')"
            />
            <button
              v-if="settings.settings.curseforge_api_key_set"
              class="mirror-btn"
              @click="clearKey('curseforge')"
            >
              {{ t("settings.content.keyClear") }}
            </button>
          </div>
          <p class="hint">
            <b>
              {{
                settings.settings.curseforge_api_key_set
                  ? t("settings.content.keyStored", {
                      hint: settings.settings.curseforge_api_key_hint,
                    })
                  : t("settings.content.keyMissing")
              }}
            </b>
            {{ t("settings.content.curseforgeHint") }}
          </p>
        </div>
        <div class="card glass" id="content-mirror">
          <h3>{{ t("settings.content.mirror") }}</h3>
          <div class="mirror-list">
            <button
              v-for="m in contentMirrors"
              :key="m.id"
              type="button"
              class="mirror-item"
              :class="{ active: settings.settings.content_mirror === m.id }"
              @click="selectContentMirror(m.id)"
            >
              <span class="mirror-main">
                <span class="mirror-name">{{ m.label }}</span>
                <span class="mirror-base">{{ m.base || t("settings.content.mirrorOfficial") }}</span>
              </span>
              <span class="mirror-side">
                <span
                  v-if="contentMirrorLatency[m.id] !== undefined"
                  class="mirror-ms"
                  :class="{ bad: contentMirrorLatency[m.id] === null }"
                >
                  {{ contentMirrorLatency[m.id] === null ? t("settings.download.mirrorUnavailable") : `${contentMirrorLatency[m.id]} ms` }}
                </span>
                <span
                  class="mirror-btn"
                  :class="{ disabled: testingContentMirror === m.id }"
                  @click.stop="testContentMirror(m.id, m.base)"
                >
                  {{ testingContentMirror === m.id ? t("settings.download.testing") : t("settings.download.test") }}
                </span>
              </span>
            </button>
            <div
              class="mirror-custom"
              :class="{ active: settings.settings.content_mirror === 'custom' }"
              role="button"
              tabindex="0"
              @click="selectContentMirror('custom')"
              @keydown.enter.prevent="selectContentMirror('custom')"
              @keydown.space.prevent="selectContentMirror('custom')"
            >
              <span class="mirror-custom-head">{{ t("settings.content.mirrorCustom") }}</span>
              <input
                v-model="settings.settings.content_mirror_custom"
                class="text-input mono"
                placeholder="https://your-mirror.example.com"
                @input="onCustomContentMirrorInput"
              />
              <span
                class="mirror-btn"
                :class="{ disabled: testingContentMirror === 'custom' || !settings.settings.content_mirror_custom }"
                @click="testContentMirror('custom', settings.settings.content_mirror_custom)"
              >
                {{ testingContentMirror === 'custom' ? t("settings.download.testing") : t("settings.download.test") }}
              </span>
            </div>
          </div>
          <p class="hint">
            {{ t("settings.content.mirrorHintLead") }}
            <b>{{ t("settings.content.mirrorHintBold") }}</b>{{ t("settings.content.mirrorHintTail") }}
          </p>
        </div>
        <div class="card glass" id="content-proxy">
          <h3>{{ t("settings.content.proxy") }}</h3>
          <div class="proxy-row">
            <div ref="proxyModeSegRef" v-segmented="{ label: t('settings.content.proxy') }" class="seg qx-seg qx-seg--sm">
              <div class="qx-seg__indicator" :style="proxyModeSegStyle"></div>
              <button
                v-for="m in proxyModes"
                :key="m.id"
                :class="{ active: settings.settings.proxy_mode === m.id }"
                @click="selectProxyMode(m.id)"
              >
                {{ m.label }}
              </button>
            </div>
            <button
              class="mirror-btn proxy-test-btn"
              :class="{ disabled: testingProxy }"
              @click="testProxy"
            >
              {{ testingProxy ? t("settings.download.testing") : t("settings.content.proxyTest") }}
            </button>
          </div>
          <input
            v-if="settings.settings.proxy_mode === 'custom'"
            v-model="settings.settings.proxy"
            class="text-input mono"
            :placeholder="t('settings.content.proxyPlaceholder')"
            @input="onCustomProxyInput"
          />
          <p class="hint">
            {{
              settings.settings.proxy_mode === "system"
                ? t("settings.content.proxySystem")
                : settings.settings.proxy_mode === "direct"
                  ? t("settings.content.proxyDirect")
                  : t("settings.content.proxyCustom")
            }}
          </p>
        </div>
        <div class="card glass" id="content-translate">
          <h3>{{ t("settings.content.translate") }}</h3>
          <n-select
            v-model:value="settings.settings.translate_provider"
            :options="translateOptions"
            size="small"
            class="tb-select"
            @update:value="settings.save()"
          />
          <template v-if="settings.settings.translate_provider === 'custom'">
            <input
              v-model="settings.settings.translate_api_base"
              class="text-input mono"
              style="margin-top: 10px"
              :placeholder="t('settings.content.translateApiPlaceholder')"
              @change="settings.save()"
            />
            <div class="proxy-row" style="margin-top: 10px">
              <input
                v-model="trKey"
                class="text-input mono"
                type="password"
                :placeholder="t('settings.content.translateKeyPlaceholder')"
                @change="saveKey('translate')"
              />
              <button
                v-if="settings.settings.translate_api_key_set"
                class="mirror-btn"
                @click="clearKey('translate')"
              >
                {{ t("settings.content.keyClear") }}
              </button>
            </div>
            <div class="proxy-row" style="margin-top: 10px">
              <input
                v-model="settings.settings.translate_api_model"
                class="text-input mono"
                :placeholder="t('settings.content.translateModelPlaceholder')"
                @change="settings.save()"
              />
              <button
                class="mirror-btn proxy-test-btn"
                :class="{ disabled: testingTranslate }"
                @click="testTranslate"
              >
                {{ testingTranslate ? t("settings.download.testing") : t("settings.content.proxyTest") }}
              </button>
            </div>
            <p class="hint">
              <b>
                {{
                  settings.settings.translate_api_key_set
                    ? t("settings.content.keyStored", {
                        hint: settings.settings.translate_api_key_hint,
                      })
                    : t("settings.content.keyMissing")
                }}
              </b>
              {{ t("settings.content.translateCustomHint") }}
            </p>
          </template>
          <p v-else-if="settings.settings.translate_provider === 'baidu_web'" class="hint">
            {{ t("settings.content.translateBaiduHint") }}
          </p>
          <p v-else class="hint">{{ t("settings.content.translateDefaultHint") }}</p>
          <div class="proxy-row" style="margin-top: 12px">
            <button
              class="mirror-btn proxy-test-btn"
              :class="{ disabled: clearingCacheService !== null }"
              @click="clearTranslations('default')"
            >
              {{ clearingCacheService === "default" ? t("settings.content.clearing") : t("settings.content.clearDefaultCache") }}
            </button>
            <button
              class="mirror-btn proxy-test-btn"
              :class="{ disabled: clearingCacheService !== null }"
              @click="clearTranslations('custom')"
            >
              {{ clearingCacheService === "custom" ? t("settings.content.clearing") : t("settings.content.clearCustomCache") }}
            </button>
          </div>
          <p class="hint">{{ t("settings.content.cacheHint") }}</p>
          <div class="switch-row">
            <span>{{ t("settings.content.autoBody") }}</span>
            <button
              class="toggle"
              :class="{ on: settings.settings.body_translate_auto }"
              role="switch"
              :aria-checked="settings.settings.body_translate_auto"
              @click="settings.patch({ body_translate_auto: !settings.settings.body_translate_auto })"
            >
              <span class="knob"></span>
            </button>
          </div>
          <p class="hint">
            {{ t("settings.content.autoBodyHint") }}
          </p>
        </div>
      </div>
    </div>
  </div>
</template>
