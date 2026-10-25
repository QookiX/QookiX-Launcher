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
  // 标题栏「刷新」按钮触发信号：TitleBar 自增，BrowseView 监听后执行刷新
  const refreshSignal = ref(0);

  function isFresh() {
    return news.value.length > 0 && Date.now() - loadedAt.value < CACHE_TTL;
  }

  async function load(force = false) {
    if (!force && isFresh()) return;
    const hasCache = news.value.length > 0;
    if (!hasCache || force) loading.value = true;
    try {
      news.value = await api.fetchNews();
      loadedAt.value = Date.now();
    } finally {
      loading.value = false;
    }
  }

  return { news, loading, isFresh, load, active, refreshSignal };
});
