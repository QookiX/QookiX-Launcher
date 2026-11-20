import { defineStore } from "pinia";
import { api } from "../api";
import type { JavaInfo, Settings } from "../types";

const JAVA_CACHE_TTL = 60_000;

const savedSettings = new WeakMap<object, Settings>();
const pendingSettings = new WeakMap<object, Promise<unknown>>();

function enqueue<T>(store: object, save: () => Promise<T>): Promise<T> {
  const previous = pendingSettings.get(store) ?? Promise.resolve();
  const next = previous.catch(() => {}).then(save);
  pendingSettings.set(store, next);
  return next;
}

function copySettings(settings: Settings): Settings {
  return JSON.parse(JSON.stringify(settings)) as Settings;
}

function changedSettings(current: Settings, previous?: Settings | null): Record<string, unknown> {
  const prior = previous as Record<string, unknown> | null | undefined;
  return Object.fromEntries(
    Object.entries(current).filter(([key, value]) => JSON.stringify(value) !== JSON.stringify(prior?.[key])),
  );
}

export const useSettingsStore = defineStore("settings", {
  state: () => ({
    settings: null as Settings | null,
    loading: false,
    /** Shared Java detection results (avoid repeated expensive scans) */
    javaCandidates: [] as JavaInfo[],
    javaFetchedAt: 0,
  }),
  getters: {
    maxMemory: (s) => s.settings?.max_memory_mb ?? 4096,
    bestJava: (s): JavaInfo | null => {
      if (!s.javaCandidates.length) return null;
      return s.javaCandidates.reduce((a, b) => (b.major > a.major ? b : a));
    },
  },
  actions: {
    load() {
      return enqueue(this, async () => {
        this.loading = true;
        try {
          const received = await api.getSettings();
          if (received) {
            const dirty = this.settings && changedSettings(this.settings, savedSettings.get(this));
            savedSettings.set(this, received);
            this.settings = { ...received, ...dirty };
          }
        } finally {
          this.loading = false;
        }
      });
    },
    patch(patch: Record<string, unknown>) {
      return enqueue(this, async () => {
        const baseline = this.settings && copySettings(this.settings);
        const received = await api.setSettings(patch);
        if (!received) return;
        const dirty = this.settings && changedSettings(this.settings, savedSettings.get(this));
        for (const key of Object.keys(patch)) {
          if (!dirty || JSON.stringify(dirty[key]) !== JSON.stringify((baseline as Record<string, unknown> | null)?.[key])) continue;
          // Explicit patches win for their own fields unless the form changed
          // that field while the request was in flight.
          delete dirty[key];
        }
        savedSettings.set(this, received);
        this.settings = { ...received, ...dirty };
      });
    },
    save() {
      return enqueue(this, async () => {
        if (!this.settings) return;
        const submitted = copySettings(this.settings);
        const patch = changedSettings(submitted, savedSettings.get(this));
        if (!Object.keys(patch).length) return;
        const received = await api.setSettings(patch);
        if (!received) return;
        const editsWhileSaving = changedSettings(this.settings, submitted);
        savedSettings.set(this, received);
        this.settings = { ...received, ...editsWhileSaving };
      });
    },
    /** Fetch detected Java, reusing the cache unless `force` (查找 Java). */
    async loadJava(force = false): Promise<JavaInfo[]> {
      if (
        !force &&
        this.javaCandidates.length &&
        Date.now() - this.javaFetchedAt < JAVA_CACHE_TTL
      ) {
        return this.javaCandidates;
      }
      try {
        const res = await api.detectJava(force);
        this.javaCandidates = res.candidates;
        this.javaFetchedAt = Date.now();
      } catch {
        // keep whatever we had
      }
      return this.javaCandidates;
    },
  },
});
