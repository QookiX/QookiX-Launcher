import { defineStore } from "pinia";
import type { ProjectHit } from "../types";

/**
 * 收藏夹。
 *
 * 存的是「卡片要用的那几个字段」的快照，而不是只存 provider:id —— 这样收藏夹
 * 不依赖上游接口也能直接渲染（平台抽风 / 断网时自己的清单照样看得到），
 * 而且列表排序（下载量、更新时间）也不需要再回源。
 *
 * 持久化走 localStorage（WebView2 会落到应用数据目录，重启仍在）。
 * 没有走 SQLite：收藏属于纯前端数据，pins 走数据库是因为它要在启动流程里
 * 和实例/服务器对齐；收藏夹没有这个约束，没必要为它加一条 Rust 命令 + 建表。
 */
export interface FavoriteItem {
  /** `${provider}:${id}`，收藏夹内的唯一键 */
  key: string;
  provider: ProjectHit["provider"];
  id: string;
  slug: string;
  title: string;
  description: string;
  author: string;
  icon_url: string;
  downloads: number;
  follows: number;
  project_type: string;
  categories: string[];
  updated: string;
  /** 收藏时间戳（毫秒），「最近收藏」排序用 */
  addedAt: number;
}

const STORAGE_KEY = "qookix.favorites";

export function favoriteKey(p: Pick<ProjectHit, "provider" | "id">): string {
  return `${p.provider}:${p.id}`;
}

/** 把收藏项还原成卡片能直接吃的 ProjectHit（详情页没存到的字段留空即可） */
export function favoriteToHit(it: FavoriteItem): ProjectHit {
  return {
    provider: it.provider,
    id: it.id,
    slug: it.slug,
    title: it.title,
    description: it.description,
    author: it.author,
    downloads: it.downloads,
    follows: it.follows,
    icon_url: it.icon_url,
    project_type: it.project_type,
    categories: it.categories,
    latest_version: "",
    game_versions: [],
    updated: it.updated,
    featured_image: "",
  };
}

function loadStored(): FavoriteItem[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const arr = JSON.parse(raw);
    if (!Array.isArray(arr)) return [];
    // 老数据 / 手改过的 json 里可能有半截条目，过滤掉，别让列表渲染时炸
    return (arr as FavoriteItem[]).filter(
      (it) => it && typeof it.key === "string" && it.provider && it.id && it.title,
    );
  } catch {
    return [];
  }
}

function persist(items: FavoriteItem[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(items));
  } catch {
    // 配额满 / 隐私模式：内存里仍然是好的，只是这次没落盘
  }
}

/** 云同步：云端条目的最小校验（手改过 / 老版本都可能缺字段） */
export function isFavoriteItem(it: unknown): it is FavoriteItem {
  const o = it as FavoriteItem | null;
  return !!o && typeof o.key === "string" && !!o.provider && !!o.id && !!o.title;
}

export const useFavoritesStore = defineStore("favorites", {
  state: () => ({
    items: loadStored(),
  }),
  getters: {
    count: (s) => s.items.length,
    has: (s) => (key: string) => s.items.some((i) => i.key === key),
  },
  actions: {
    isFavorite(p: Pick<ProjectHit, "provider" | "id">) {
      return this.items.some((i) => i.key === favoriteKey(p));
    },
    add(p: ProjectHit) {
      const key = favoriteKey(p);
      if (this.items.some((i) => i.key === key)) return;
      this.items.unshift({
        key,
        provider: p.provider,
        id: p.id,
        slug: p.slug,
        title: p.title,
        description: p.description,
        author: p.author,
        icon_url: p.icon_url,
        downloads: p.downloads,
        follows: p.follows,
        project_type: p.project_type,
        categories: [...(p.categories ?? [])],
        updated: p.updated,
        addedAt: Date.now(),
      });
      persist(this.items);
    },
    remove(key: string) {
      const next = this.items.filter((i) => i.key !== key);
      if (next.length !== this.items.length) {
        this.items = next;
        persist(this.items);
      }
    },
    /** 云同步：导出快照 */
    snapshot(): FavoriteItem[] {
      return this.items.map((it) => ({ ...it }));
    },
    /**
     * 云同步：合并云端收藏。
     * 以本地为准（同 key 不覆盖），只补本地没有的——收藏是「只增」的数据，
     * 直接覆盖会把另一台机器上刚加的条目吞掉。
     * @returns 新增条数
     */
    mergeFromCloud(list: unknown): number {
      if (!Array.isArray(list)) return 0;
      const seen = new Set(this.items.map((i) => i.key));
      let added = 0;
      for (const raw of list) {
        if (!isFavoriteItem(raw) || seen.has(raw.key)) continue;
        seen.add(raw.key);
        this.items.push(raw);
        added += 1;
      }
      if (added > 0) {
        this.items.sort((a, b) => (b.addedAt ?? 0) - (a.addedAt ?? 0));
        persist(this.items);
      }
      return added;
    },
    /** @returns 收藏后的状态：true = 已收藏，false = 已取消 */
    toggle(p: ProjectHit): boolean {
      const key = favoriteKey(p);
      if (this.items.some((i) => i.key === key)) {
        this.remove(key);
        return false;
      }
      this.add(p);
      return true;
    },
    clear() {
      this.items = [];
      persist(this.items);
    },
  },
});
