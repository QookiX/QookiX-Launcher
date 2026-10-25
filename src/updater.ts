import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { relaunch } from "@tauri-apps/plugin-process";
import { useTasksStore } from "./stores/tasks";
import { error as devError } from "./utils/logger";
import changelogRaw from "../CHANGELOG.md?raw";

/**
 * 应用自更新检查结果（由 Rust 端 `updater::check_for_update` 返回）。
 * `available` 为 `false` 表示当前所选更新源没有新版本。
 */
export interface UpdateInfo {
  available: boolean;
  version: string | null;
  currentVersion: string | null;
  body: string | null;
  downloadUrl: string | null;
  /** 实际使用的更新源："bucket" | "github" */
  source: "bucket" | "github";
  /** 当前版本低于 latest.json 的 minVersion 时为 true：必须更新、不可忽略。 */
  forceUpdate: boolean;
  /** 触发强制更新的最低版本（展示用）。 */
  forceMinVersion: string | null;
}

/**
 * `true` once an update has been downloaded + installed and is only waiting for
 * a restart to take effect. The title bar shows a「重启以更新」button while this
 * is set — the app never restarts on its own.
 */
export const updateReady = ref(false);
/** Version that was just downloaded (shown in the title-bar button tooltip). */
export const updateReadyVersion = ref<string | null>(null);
/** `true` while `install()` is running (guards double clicks on the button). */
export const updateInstalling = ref(false);

/**
 * 当检测到强制更新时填入，触发全屏锁死遮罩。`null` 表示无强制更新。
 * 遮罩组件 `ForceUpdateOverlay.vue` watch 这个 ref。
 */
export const forceUpdateInfo = ref<UpdateInfo | null>(null);

export interface UpdateProgress {
  message: string;
  /** 0..1 download fraction while downloading */
  fraction?: number;
  /** bytes downloaded so far (optional, used for the speed indicator) */
  bytesDone?: number;
  /** total bytes to download (optional) */
  bytesTotal?: number;
}
type ProgressFn = (p: UpdateProgress) => void;

/**
 * Check whether an update is available on the **currently selected update
 * source** (存储桶 / GitHub). Returns the update info, or `null` when there is
 * nothing to update or the updater is not configured (errors are swallowed
 * silently so a network blip never blocks the launcher).
 */
export async function peekUpdate(strict = false): Promise<UpdateInfo | null> {
  try {
    const info = await invoke<UpdateInfo>("check_for_update");
    return info.available ? info : null;
  } catch (err) {
    // 默认吞掉错误：启动时的静默检查不希望网络抖动阻塞启动。
    // strict（如用户手动点「检查更新」）时把真实错误抛给调用方，
    // 以便区分「确实没有新版本」和「更新源不可用」，避免误报已是最新。
    if (strict) throw err;
    return null;
  }
}

/**
 * Fetch the changelog JSON from the update source (China mirror first, GitHub
 * fallback). Returns the raw JSON text, or `null` on failure.
 *
 * 由 CI 的 `scripts/generate-changelog-json.mjs` 从 `CHANGELOG.md` 生成，
 * 随 Release 一起上传为 `changelog.json`（**只含最近 5 个版本**）。
 */
export async function fetchChangelog(): Promise<string | null> {
  try {
    return await invoke<string>("fetch_changelog");
  } catch {
    return null;
  }
}

/** 更新日志的一条记录（与 `changelog.json` 的字段对齐）。 */
export interface ChangelogEntry {
  category: string;
  text: string;
}

/** 更新日志里的一个版本。 */
export interface ChangelogRelease {
  version: string;
  date: string;
  entries: ChangelogEntry[];
}

/**
 * 根据条目文案里的关键词推断分类，决定条目标签的颜色
 * （修复=红、新增=绿、优化=蓝、重构=紫、其他=灰）。
 * 更新弹窗与关于页的更新日志共用，保证配色一致。
 */
export function entryTypeClass(text: string): string {
  if (/修复|修正|解决|补回/.test(text)) return "cat-fix";
  if (/新增|添加|增加|支持|接入/.test(text)) return "cat-feature";
  if (/优化|改进|提升|统一|完善|加强|增强/.test(text)) return "cat-improve";
  if (/重构|调整|改用|取代|移除|删除|过滤/.test(text)) return "cat-refactor";
  return "cat-other";
}

/**
 * 版本号比较：按 `.` 分段数值比较（`0.10.0 > 0.9.0`，字符串比较会搞错）。
 * 无法解析的段按 0 处理，返回 -1 / 0 / 1。前置的 `v` / `V` 会被自动剔除。
 */
export function compareVersions(a: string, b: string): number {
  const parse = (s: string) =>
    s
      .trim()
      .replace(/^[vV]/, "")
      .split(/[.+-]/)
      .map((seg) => Number.parseInt(seg, 10) || 0);
  const pa = parse(a);
  const pb = parse(b);
  const n = Math.max(pa.length, pb.length);
  for (let i = 0; i < n; i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d !== 0) return d < 0 ? -1 : 1;
  }
  return 0;
}

/**
 * 解析 `CHANGELOG.md`（本地兜底用）。只取 `## [版本] - 日期` 小节里的
 * `编号. **模块**：说明` 条目，默认最多 5 个版本（与 changelog.json 对齐）。
 */
export function parseChangelogMarkdown(md: string, limit = 5): ChangelogRelease[] {
  const lines = md.replace(/\r\n/g, "\n").split("\n");
  const releases: ChangelogRelease[] = [];
  let current: ChangelogRelease | null = null;
  for (const line of lines) {
    const heading = /^##\s+\[(.*?)\]\s*-\s*(\d{4}-\d{2}-\d{2})/.exec(line);
    if (heading) {
      const [, version, date] = heading;
      if (/unreleased/i.test(version)) { current = null; continue; }
      current = { version, date, entries: [] };
      releases.push(current);
      continue;
    }
    if (!current) continue;
    const entry = /^\d+[.)]\s+\*\*(.+?)\*\*[:：]\s*(.+)$/.exec(line.trim());
    if (entry) {
      current.entries.push({ category: entry[1].trim(), text: entry[2].trim() });
    }
  }
  return releases.slice(0, limit);
}

/**
 * 取更新日志：远端 `changelog.json` 优先（国内镜像 → GitHub），失败回退到
 * 打进包的 `CHANGELOG.md`。两者都只含最近 5 个版本。
 */
export async function loadChangelogReleases(): Promise<ChangelogRelease[]> {
  const remote = await fetchChangelog();
  if (remote) {
    try {
      const parsed = JSON.parse(remote) as ChangelogRelease[];
      if (Array.isArray(parsed) && parsed.length > 0) return parsed;
    } catch {
      /* 不是合法 JSON：走本地兜底 */
    }
  }
  return parseChangelogMarkdown(changelogRaw);
}

/**
 * 取「当前版本（不含）→ 最新版本（含）」之间跨过的所有版本，按版本倒序。
 *
 * `releases` 通常来自 `changelog.json`（只有最近几个版本）——比它更旧的版本
 * 拿不到就直接省略；`currentVersion` 缺失时退化为只返回最新一版。结果为空
 * （例如当前版本比日志还新）时同样回退到最新一版，保证弹窗里总有内容。
 */
export function releasesInRange(
  releases: ChangelogRelease[],
  currentVersion: string | null | undefined,
  latestVersion: string | null | undefined,
): ChangelogRelease[] {
  if (releases.length === 0) return [];
  const latest = latestVersion ?? releases[0].version;
  const within = releases.filter((r) => compareVersions(r.version, latest) <= 0);
  if (!currentVersion) return within.slice(0, 1);
  const out = within.filter((r) => compareVersions(r.version, currentVersion) > 0);
  return out.length > 0 ? out : within.slice(0, 1);
}

/**
 * Download the pending update from the selected source **without installing
 * it**. The download+signature-check happen on the Rust side, which streams
 * progress back through the `update://progress` event.
 *
 * Important: on Windows, installing means running the NSIS installer, which
 * forcibly kills the running process and then relaunches the app (`/R`). So the
 * install step must NOT happen right after the download — `applyUpdateNow()`
 * does that only once the user clicks「重启以更新」.
 *
 * Reports progress through `onStatus` AND registers a task in the Download
 * Center (`tasks` store). Returns `true` when an update was downloaded.
 */
export async function downloadUpdate(onStatus?: ProgressFn): Promise<boolean> {
  const tasks = useTasksStore();
  const taskId = Date.now();
  let registered = false;
  // The updater emits a progress event for *every* network chunk, which can be
  // hundreds per second — throttle the store writes (and the resulting UI
  // re-renders) to ~4/s so the Download Center stays responsive.
  let lastPush = 0;

  const emit = (p: UpdateProgress, force = false) => {
    onStatus?.(p);
    if (!registered) return;
    const now = Date.now();
    if (!force && p.bytesDone != null && now - lastPush < 250) return;
    lastPush = now;
    tasks.upsert(taskId, (t) => {
      t.message = p.message;
      if (p.bytesTotal && p.bytesTotal > 0) {
        t.bytesTotal = Math.max(t.bytesTotal, p.bytesTotal);
      }
      if (p.bytesDone != null) {
        t.activity = "download";
        t.bytesDone = Math.max(t.bytesDone, p.bytesDone);
        // feed the shared rolling-window speed sampler, so the Download Center
        // shows a live MB/s figure just like the game downloads do
        tasks.sampleSpeed(t, p.bytesDone);
      }
      if (p.fraction != null) t.fraction = p.fraction;
    });
  };

  let unlisten: UnlistenFn | undefined;
  let version: string | null = null;
  try {
    unlisten = await listen<{ downloaded: number; total: number | null }>(
      "app-update-progress",
      (event) => {
        const { downloaded, total } = event.payload;
        const fraction = total && total > 0 ? downloaded / total : undefined;
        emit({
          message:
            fraction != null
              ? `下载中 ${Math.round(fraction * 100)}%…`
              : "下载中…",
          fraction,
          bytesDone: downloaded,
          bytesTotal: total ?? undefined,
        });
      }
    );

    registered = true;
    tasks.upsert(taskId, (t) => {
      t.activity = "download";
      t.source = "启动器更新";
      t.stage = "download";
      t.message = "准备下载更新…";
    });

    const ok = await invoke<boolean>("download_update");
    if (!ok) return false;

    const info = await peekUpdate();
    version = info?.version ?? null;

    tasks.upsert(taskId, (t) => {
      t.stage = "done";
      t.finished = true;
      t.ok = true;
      t.fraction = 1;
      t.speed = 0;
      t.samples = [];
      t.message = `v${version ?? ""} 已下载，点击标题栏「重启以更新」安装`;
    });
    // Downloaded, not installed: the user decides when to restart.
    updateReadyVersion.value = version;
    updateReady.value = true;
    return true;
  } catch (err) {
    // Surface the real reason (404, signature mismatch, permissions...) so the
    // UI can show something actionable instead of a generic "update failed".
    devError("[updater] downloadUpdate failed:", err);
    const detail = err instanceof Error && err.message ? err.message : String(err);
    if (registered) {
      tasks.upsert(taskId, (t) => {
        t.finished = true;
        t.ok = false;
        t.fraction = 0;
        t.speed = 0;
        t.samples = [];
        t.message = `更新失败：${detail}`;
      });
    }
    throw new Error(`更新失败：${detail}`);
  } finally {
    unlisten?.();
  }
}

/**
 * Install the downloaded package and restart — what the title-bar
 *「重启以更新」button is wired to.
 *
 * Note: on Windows the installer kills this process as part of the install and
 * relaunches the app itself (the updater passes `/R`), so the `relaunchApp()`
 * below normally never runs. It is only a fallback for platforms/installers
 * that leave the process alive.
 */
export async function applyUpdateNow(): Promise<void> {
  if (updateInstalling.value) return;
  updateInstalling.value = true;
  try {
    await invoke("apply_app_update");
    await relaunchApp();
  } catch (err) {
    devError("[updater] apply_update failed:", err);
    throw err instanceof Error ? err : new Error(String(err));
  } finally {
    updateInstalling.value = false;
  }
}

/** Relaunch the app (used after an update was installed). */
export async function relaunchApp(): Promise<void> {
  try {
    await relaunch();
  } catch {
    /* ignore */
  }
}
