import { defineStore } from "pinia";
import type { EventCallback } from "@tauri-apps/api/event";
import { createTauriListenerScope } from "../composables/tauriListeners";
import { useInstancesStore } from "./instances";
import type {
  ActiveFile,
  DownloadProgressEvent,
  InstallProgressEvent,
  LaunchLogEvent,
  LaunchProgressEvent,
  LaunchStateEvent,
} from "../types";

interface LogEntry {
  stream: "out" | "err";
  line: string;
}

export interface TaskFile {
  name: string;
  ok: boolean;
}

export interface TaskEntry {
  id: number;
  stage: string;
  message: string;
  // download phase (files + bytes)
  fileDone: number;
  fileTotal: number;
  bytesDone: number;
  bytesTotal: number;
  verifiedBytes: number;
  // install phase (steps)
  stepDone: number;
  stepTotal: number;
  current?: string;
  lastCurrent?: string;
  activeFiles: ActiveFile[];
  speed: number; // bytes/sec (average over a rolling window)
  samples: { ts: number; bytes: number }[];
  files: TaskFile[];
  instanceId?: string;
  instanceName?: string;
  source?: string;
  startedAt: number;
  finished: boolean;
  cancellable?: boolean;
  cancelRequested?: boolean;
  ok?: boolean;
  activity: "download" | "install";
  /** 0..1 overall progress (used e.g. by the app updater, which has no per-file/byte accounting). */
  fraction?: number;
}

function nowMs() {
  return Date.now();
}

/** Install-side stages that are not pure downloads. */
const INSTALL_STAGES = ["manifest", "natives", "modpack-install", "done"];
const subscriptions = new WeakMap<object, { controller: AbortController; ready: Promise<void> }>();

export const useTasksStore = defineStore("tasks", {
  state: () => ({
    tasks: {} as Record<number, TaskEntry>,
    order: [] as number[],
    logs: {} as Record<string, LogEntry[]>,
    gameTasks: {} as Record<string, { taskId: number; state: "preparing" | "running" }>,
    launchOverlay: null as LaunchProgressEvent & { done: boolean } | null,
    lastExit: null as { instanceId: string; code: number | null } | null,
  }),
  getters: {
    taskList(): TaskEntry[] {
      return this.order
        .map((id) => this.tasks[id])
        .filter(Boolean)
        .sort((a, b) => b.startedAt - a.startedAt);
    },
    activeCount(): number {
      return this.taskList.filter((t) => !t.finished).length;
    },
    runningInstances(): string[] {
      return Object.keys(this.gameTasks).filter((id) => this.gameTasks[id].state === "running");
    },
    gamePreparing(): boolean {
      return Object.values(this.gameTasks).some((task) => task.state === "preparing");
    },
    gameRunning(): boolean {
      return this.runningInstances.length > 0;
    },
    runningInstance(): string | null {
      return this.runningInstances[0] ?? null;
    },
  },
  actions: {
    init() {
      const existing = subscriptions.get(this);
      if (existing) return existing.ready;

      const controller = new AbortController();
      const { signal } = controller;
      const scope = createTauriListenerScope();
      const pending: Promise<void>[] = [];
      signal.addEventListener("abort", scope.dispose, { once: true });
      function bind<T>(event: string, handle: EventCallback<T>) {
        pending.push(scope.listen<T>(event, handle).then(() => {}));
      }

      bind<InstallProgressEvent>("install://progress", (e) => {
        const p = e.payload;
        this.upsert(p.taskId, (t) => {
          if (t.finished) return;
          t.stage = p.stage;
          t.message = p.message;
          t.stepDone = p.done;
          t.stepTotal = p.total;
          t.activity = INSTALL_STAGES.includes(p.stage) ? "install" : "download";
          if (p.instanceId) t.instanceId = p.instanceId;
          if (p.instanceName) t.instanceName = p.instanceName;
          if (p.source && (p.stage !== "done" || !t.source)) t.source = p.source;
          t.cancellable = p.cancellable === true;
          if (p.stage === "done") {
            t.finished = true;
            t.ok = p.ok !== false;
            t.speed = 0;
            // flush the last tracked file
            if (t.lastCurrent) {
              t.files.push({ name: t.lastCurrent, ok: t.ok ?? true });
              t.lastCurrent = undefined;
            }
          }
        });
      });
      bind<DownloadProgressEvent>("download://progress", (e) => {
        const p = e.payload;
        this.upsert(p.taskId, (t) => {
          if (t.finished) return;
          const bytesDone = p.bytesDone ?? 0;
          if (t.stage !== p.phase || bytesDone < t.bytesDone) {
            t.samples = [];
            t.speed = 0;
          }
          t.activity = "download";
          t.stage = p.phase;
          t.fileDone = p.done;
          t.fileTotal = p.total;
          t.bytesDone = bytesDone;
          t.bytesTotal = p.bytesTotal ?? 0;
          t.verifiedBytes = p.verifiedBytes;
          t.ok = p.ok;
          if (p.activeFiles) {
            t.activeFiles = p.activeFiles;
          }
          if (p.current) {
            t.files.push({ name: p.current, ok: p.ok ?? true });
            if (t.files.length > 100) t.files.splice(0, t.files.length - 100);
            t.current = p.current;
          }
          // rolling average speed over the last ~5s with smoothing
          this.sampleSpeed(t, p.bytesDone ?? 0);
        });
      });
      bind<LaunchLogEvent>("launch://log", (e) => {
        const { instanceId, stream, line } = e.payload;
        if (!this.logs[instanceId]) this.logs[instanceId] = [];
        const buf = this.logs[instanceId];
        buf.push({ stream, line });
        if (buf.length > 4000) buf.splice(0, buf.length - 4000);
      });
      bind<LaunchProgressEvent>("launch://progress", ({ payload }) => {
        const current = this.gameTasks[payload.instanceId];
        if (current && (current.taskId > payload.taskId || current.state === "running")) return;
        this.gameTasks[payload.instanceId] = { taskId: payload.taskId, state: "preparing" };
        if (!this.launchOverlay || this.launchOverlay.taskId <= payload.taskId) {
          this.launchOverlay = { ...payload, done: false };
        }
      });
      bind<LaunchStateEvent>("launch://state", ({ payload }) => {
        const { instanceId, taskId, state, code } = payload;
        const current = this.gameTasks[instanceId];
        if (current && current.taskId > taskId) return;
        if (state === "running") {
          this.gameTasks[instanceId] = { taskId, state };
          if (this.launchOverlay?.taskId === taskId) {
            this.launchOverlay = { ...this.launchOverlay, progress: 100, done: true };
          }
        } else {
          delete this.gameTasks[instanceId];
          if (this.launchOverlay?.taskId === taskId) this.launchOverlay = null;
          if (!payload.pid) return;
          this.lastExit = { instanceId, code };
          void useInstancesStore().refresh();
        }
      });
      const ready = Promise.all(pending).then(() => {}).catch((error) => {
        if (subscriptions.get(this)?.controller === controller) this.disposeListeners();
        throw error;
      });
      subscriptions.set(this, { controller, ready });
      return ready;
    },
    dismissLaunchOverlay(taskId: number) {
      if (this.launchOverlay?.taskId === taskId) this.launchOverlay = null;
    },
    disposeListeners() {
      const binding = subscriptions.get(this);
      subscriptions.delete(this);
      binding?.controller.abort();
    },
    /**
     * Record a byte-counter sample and refresh the rolling-window average speed
     * (bytes/sec). Shared by the backend downloader and the in-frontend app
     * updater, which keeps its own byte counters.
     */
    sampleSpeed(t: TaskEntry, bytesDone: number) {
      const ts = nowMs();
      t.samples.push({ ts, bytes: bytesDone });
      t.samples = t.samples.filter((s) => ts - s.ts <= 5000);
      if (t.samples.length >= 3) {
        const first = t.samples[0];
        const last = t.samples[t.samples.length - 1];
        const dt = (last.ts - first.ts) / 1000;
        if (dt > 0.5) {
          const newSpeed = Math.max(0, (last.bytes - first.bytes) / dt);
          t.speed = t.speed > 0 ? t.speed * 0.7 + newSpeed * 0.3 : newSpeed;
        }
      }
      if (t.samples.length > 60) t.samples.splice(0, t.samples.length - 60);
    },
    upsert(id: number, patch: (t: TaskEntry) => void) {
      if (!this.tasks[id]) {
        this.tasks[id] = {
          id,
          stage: "",
          message: "",
          fileDone: 0,
          fileTotal: 0,
          bytesDone: 0,
          bytesTotal: 0,
          verifiedBytes: 0,
          stepDone: 0,
          stepTotal: 0,
          activeFiles: [],
          speed: 0,
          samples: [],
          files: [],
          startedAt: nowMs(),
          finished: false,
          activity: "download",
        };
        this.order.push(id);
        if (this.order.length > 60) {
          const old = this.order.shift();
          if (old !== undefined) delete this.tasks[old];
        }
      }
      patch(this.tasks[id]);
    },
    clearFinished() {
      const keep = this.order.filter((id) => {
        const t = this.tasks[id];
        if (t && t.finished) {
          delete this.tasks[id];
          return false;
        }
        return true;
      });
      this.order = keep;
    },
    clearAll() {
      this.tasks = {};
      this.order = [];
    },
    clearLogs(instanceId: string) {
      this.logs[instanceId] = [];
    },
  },
});
