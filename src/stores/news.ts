import { defineStore } from "pinia";
import { ref } from "vue";
import { api } from "../api";
import type { NewsItem } from "../types";

const CACHE_TTL = 10 * 60 * 1000;

export const useNewsStore = defineStore("news", () => {
  const news = ref<NewsItem[]>([]);
  const loadedAt = ref(0);
  const loading = ref(false);
  // 「新闻」标签是否激活：BrowseView 写入，TitleBar 读取以决定是否显示刷新按钮
  const active = ref(false);
  let pending: Promise<void> | undefined;

  function isFresh() {
    return loadedAt.value > 0 && Date.now() - loadedAt.value < CACHE_TTL;
  }

  function load(force = false): Promise<void> {
    if (pending) return pending;
    if (!force && isFresh()) return Promise.resolve();
    loading.value = true;
    pending = api.fetchNews().then((items) => {
      news.value = items;
      loadedAt.value = Date.now();
    }).finally(() => {
      loading.value = false;
      pending = undefined;
    });
    return pending;
  }

  function setActive(value: boolean) {
    active.value = value;
  }

  return { news, loading, load, active, setActive };
});
