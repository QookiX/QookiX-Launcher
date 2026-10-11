<script setup lang="ts">
import { h, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { NButton, useDialog, useMessage } from "naive-ui";
import { peekUpdate, downloadUpdate, updateReady, loadChangelogReleases, releasesInRange } from "../updater";
import { useSettingsStore } from "../stores/settings";
import { error as devError } from "../utils/logger";
import { useI18n } from "vue-i18n";

const dialog = useDialog();
const message = useMessage();
const router = useRouter();
const settings = useSettingsStore();
const { t } = useI18n();
const checking = ref(false);

onMounted(() => {
  // Check shortly after startup so the UI is ready and the home screen is shown.
  const t = setTimeout(() => runCheck(), 2000);
  // stop the check if the window is closed quickly
  window.addEventListener("beforeunload", () => clearTimeout(t));
});

async function runCheck() {
  if (checking.value) return;
  // 已经下载好、只等重启：别再重复下载同一版本
  if (updateReady.value) return;
  checking.value = true;
  try {
    if (!settings.settings) {
      try {
        await settings.load();
      } catch {
        /* ignore */
      }
    }
    const update = await peekUpdate();
    if (!update) return;
    // 用户此前点击过「忽略此版本」且仍是同一版本：不再弹窗，避免每次启动都打扰。
    if (settings.settings?.dismissed_update_version === update.version) return;

    // 自动更新开启：后台静默下载安装，不弹窗、不打断当前页面。
    // 下载完也只是点亮标题栏的「重启以更新」，绝不自动重启。
    if (settings.settings?.auto_update) {
      void doInstall(true);
      return;
    }

    // 更新内容：列出「当前版本 → 最新版本」之间跨过的所有版本。数据来自
    // changelog.json（只含最近几个版本），更旧的版本拿不到就直接省略。
    const notes = releasesInRange(
      await loadChangelogReleases(),
      update.currentVersion,
      update.version,
    );

    // naive-ui 的对话框只内置 positive/negative 两个按钮，这里用 action 自定义
    // 三个按钮，额外提供「忽略此版本」。
    let dlg: { destroy: () => void } | null = null;
    const close = () => {
      dlg?.destroy();
      dlg = null;
    };
    dlg = dialog.warning({
      title: t("updaterCheck.newVersionTitle"),
      // 对话框内容由 render 函数产出，拿不到组件的 scoped 样式，故用内联样式
      content: () =>
        h("div", { style: "display:flex;flex-direction:column;gap:12px;" }, [
          h(
            "p",
            { style: "margin:0;" },
            t("updaterCheck.newVersionContent", { version: update.version }),
          ),
          notes.length > 0
            ? h(
                "div",
                {
                  style:
                    "border:1px solid var(--border);border-radius:10px;padding:10px 12px;" +
                    "max-height:38vh;overflow-y:auto;",
                },
                [
                  h(
                    "div",
                    { style: "font-size:13px;font-weight:700;color:var(--text-1);margin-bottom:8px;" },
                    t("updaterCheck.releaseNotes"),
                  ),
                  ...notes.flatMap((r) => [
                    h(
                      "div",
                      { style: "font-size:12px;font-weight:700;color:var(--text-2);margin:12px 0 6px;" },
                      `v${r.version}`,
                    ),
                    ...r.entries.map((e) =>
                      h("div", { style: "display:flex;gap:6px;margin-bottom:6px;" }, [
                        h(
                          "span",
                          {
                            style:
                              "flex:none;font-size:10px;font-weight:600;padding:1px 6px;border-radius:6px;" +
                              "background:var(--accent-soft);color:var(--accent);",
                          },
                          e.category,
                        ),
                        h(
                          "span",
                          { style: "font-size:12.5px;line-height:1.5;color:var(--text-2);" },
                          e.text,
                        ),
                      ]),
                    ),
                  ]),
                ],
              )
            : null,
        ]),
      action: () =>
        h("div", { style: "display:flex; gap:8px; justify-content:flex-end;" }, [
          h(
            NButton,
            {
              size: "small",
              quaternary: true,
              onClick: () => {
                close();
                void dismiss(update.version ?? "");
              },
            },
            { default: () => t("updaterCheck.ignoreVersion") }
          ),
          h(NButton, { size: "small", ghost: true, onClick: close }, () => t("updaterCheck.later")),
          h(
            NButton,
            {
              size: "small",
              type: "primary",
              onClick: () => {
                close();
                void doInstall();
              },
            },
            { default: () => t("updaterCheck.downloadAndUpdate") }
          ),
        ]),
    });
  } finally {
    checking.value = false;
  }
}

/** 点击「忽略此版本」：记录版本号，此后启动时不再提示该版本。 */
async function dismiss(version: string) {
  try {
    await settings.patch({ dismissed_update_version: version });
  } catch {
    /* 保存失败则下次继续提示，不影响使用 */
  }
}

/**
 * @param auto 自动更新路径：后台下载，不跳转页面、不弹窗询问。
 */
async function doInstall(auto = false) {
  // 手动触发时才跳转到下载中心看进度；自动更新在后台跑，不打断用户。
  if (!auto) router.push("/downloads");
  try {
    const downloaded = await downloadUpdate();
    if (!downloaded) return;
    // 只下载不安装：安装与重启由标题栏「重启以更新」按钮触发。
    message.success(t("updaterCheck.downloadedHint"));
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    message.error(detail || t("updaterCheck.updateFailed"));
    devError("[updater] install error:", err);
  }
}
</script>

<template>
  <!-- Invisible host; runs the auto-update check against the providers above. -->
  <div style="display: none"></div>
</template>
