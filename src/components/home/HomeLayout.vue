<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, provide, reactive, watch } from "vue";
import { useRouter } from "vue-router";
import { useDialog, useMessage } from "naive-ui";
import { useI18n } from "vue-i18n";
import { convertFileSrc } from "@tauri-apps/api/core";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import HomeNode from "./HomeNode.vue";
import { HOME_CTX, type HomeContext } from "../../home/context";
import {
  formatDate,
  formatTime,
  interpolate,
  loadUserVars,
  nodeVisible,
  saveUserVars,
} from "../../home/layout";
import type { HomeAction, HomeLayout, HomeVars } from "../../home/types";
import { useInstancesStore } from "../../stores/instances";
import { useAccountsStore } from "../../stores/accounts";
import { copyText } from "../../utils/clipboard";
import { api } from "../../api";

const props = defineProps<{ layout: HomeLayout; homePath?: string }>();
const emit = defineEmits<{ reload: [] }>();

const router = useRouter();
const message = useMessage();
const dialog = useDialog();
const { t } = useI18n();
const instances = useInstancesStore();
const accounts = useAccountsStore();

/** 内置占位符，不写进 localStorage（每次按当前状态重新计算） */
const BUILTIN_KEYS = new Set(["user", "instance", "date", "time", "homePath"]);

const vars = reactive<HomeVars>({});

const currentInstance = computed(
  () => [...instances.instances].sort((a, b) => (b.last_played ?? 0) - (a.last_played ?? 0))[0],
);

function interp(text: string | undefined): string {
  return interpolate(text ?? "", vars);
}

function resetVars() {
  for (const key of Object.keys(vars)) delete vars[key];
  Object.assign(vars, {
    user: accounts.accounts[0]?.username ?? t("home.layout.guest"),
    instance: currentInstance.value?.name ?? t("home.layout.noInstanceName"),
    date: formatDate(),
    time: formatTime(),
    homePath: props.homePath ?? "",
    ...(props.layout.variables ?? {}),
    ...loadUserVars(),
  });
}

/** 只持久化用户变量，内置占位符不落盘 */
function persistVars() {
  const out: HomeVars = {};
  for (const [key, value] of Object.entries(vars)) {
    if (!BUILTIN_KEYS.has(key)) out[key] = value;
  }
  saveUserVars(out);
}

// 布局换了一份（重载 / 恢复默认）：重建变量表，但保留用户自己改过的值
watch(() => props.layout, resetVars, { immediate: true });
watch(() => props.homePath, (p) => { if (p) vars.homePath = p; });
// 账号 / 实例变化时，{user} 与 {instance} 跟着变
watch(() => accounts.accounts[0]?.username, (name) => { vars.user = name ?? t("home.layout.guest"); });
watch(() => currentInstance.value?.name, (name) => { vars.instance = name ?? t("home.layout.noInstanceName"); });

// 每分钟刷新一次 {time}，让占位符不至于停在打开页面那一刻
let clock = 0;
onMounted(() => {
  clock = window.setInterval(() => {
    vars.date = formatDate();
    vars.time = formatTime();
  }, 60_000);
});
onBeforeUnmount(() => window.clearInterval(clock));

function imgSrc(src: string | undefined): string {
  if (!src) return "";
  const value = interp(src);
  if (/^(https?:|data:|blob:|file:)/i.test(value)) return value;
  try {
    return convertFileSrc(value);
  } catch {
    return value;
  }
}

async function launch(action: HomeAction) {
  const name = interp(action.instance);
  const target = name
    ? instances.instances.find((i) => i.name === name) ?? instances.instances.find((i) => i.id === name)
    : currentInstance.value;
  if (!target) {
    message.info(t("home.layout.launchNoInstance"));
    return;
  }
  if (!accounts.accounts.length) {
    message.warning(t("home.layout.launchNoAccount"));
    accounts.showManager = true;
    return;
  }
  try {
    await instances.launch(
      target.id,
      action.world ? interp(action.world) : undefined,
      action.server ? interp(action.server) : undefined,
    );
    message.success(t("home.layout.launched", { name: target.name }));
  } catch (e) {
    message.error(String(e));
  }
}

async function runOne(action: HomeAction) {
  switch (action.type) {
    case "open-url": {
      const url = interp(action.url);
      if (!url) return;
      await openUrl(url).catch((e: unknown) =>
        message.error(t("home.layout.openFailed", { error: String(e) })),
      );
      return;
    }
    case "copy": {
      const ok = await copyText(interp(action.text));
      if (ok) message.success(t("home.layout.copied"));
      else message.error(t("home.layout.copyFailed"));
      return;
    }
    case "dialog": {
      dialog.info({
        title: interp(action.title) || t("home.layout.dialogTitle"),
        content: interp(action.content),
        positiveText: t("common.ok"),
      });
      return;
    }
    case "navigate": {
      const to = interp(action.to);
      if (to) void router.push(to);
      return;
    }
    case "open-path": {
      const path = interp(action.path);
      if (!path) return;
      try {
        await openPath(path);
      } catch {
        // 没有默认打开程序时退化为「在文件管理器中定位」
        try {
          await api.revealPath(path);
        } catch (e) {
          message.error(t("home.layout.openFailed", { error: String(e) }));
        }
      }
      return;
    }
    case "launch":
      await launch(action);
      return;
    case "set-var": {
      if (!action.name) return;
      vars[action.name] = interp(action.value ?? "");
      persistVars();
      return;
    }
    case "reload":
      emit("reload");
      return;
    default:
      message.warning(
        t("home.layout.unknownAction", { type: String((action as { type: string }).type) }),
      );
  }
}

function run(action: HomeAction | HomeAction[] | undefined) {
  if (!action) return;
  const list = Array.isArray(action) ? action : [action];
  for (const item of list) void runOne(item);
}

const ctx: HomeContext = {
  vars,
  interp,
  visible: (node) => nodeVisible(node, vars),
  run,
  imgSrc,
};
provide(HOME_CTX, ctx);
</script>

<template>
  <div class="home-layout">
    <HomeNode v-for="(node, i) in layout.nodes" :key="`${i}-${node.type}`" :node="node" />
  </div>
</template>

<style scoped>
.home-layout {
  display: flex;
  flex-direction: column;
  gap: var(--gap-page);
}
</style>
