<script setup lang="ts">
import { onMounted } from "vue";
import { useMessage } from "naive-ui";
import { useI18n } from "vue-i18n";
import { openUrl } from "@tauri-apps/plugin-opener";
import { IconRefresh } from "../components/icons";
import { useNewsStore } from "../stores/news";
import { fmtDateShort as fmtDate } from "../utils/format";

const { t } = useI18n();
const message = useMessage();
const newsStore = useNewsStore();

// 资讯卡片逐个进场（同首页 pin-card / 内容页卡片的做法）：延迟由内联 style 下发。
// 数量多时自动收紧步长，保证整批级联在 ~0.7s 内跑完；刷新换一批新闻时重播。
function cardDelay(i: number): string {
  const step = Math.min(0.045, 0.6 / Math.max(1, newsStore.news.length));
  return `${(0.06 + i * step).toFixed(3)}s`;
}


async function refresh() {
  try {
    await newsStore.load(true);
  } catch (e) {
    message.error(String(e));
  }
}

onMounted(async () => {
  try {
    await newsStore.load();
  } catch (e) {
    message.error(String(e));
  }
});
</script>

<template>
  <div class="news-view">
    <div class="news-header glass">
      <h1>{{ t("news.title") }}</h1>
      <button class="refresh-btn" :disabled="newsStore.loading" @click="refresh">
        <IconRefresh class="btn-icon" />
        {{ newsStore.loading ? t("news.refreshing") : t("common.refresh") }}
      </button>
    </div>
    <div v-if="newsStore.loading && !newsStore.news.length" class="loading glass">{{ t("common.loading") }}</div>
    <div v-else-if="!newsStore.news.length" class="empty glass">{{ t("news.empty") }}</div>
    <div v-else class="news-list">
      <div
        v-for="(n, i) in newsStore.news"
        :key="n.url"
        class="news-card glass"
        :style="{ animationDelay: cardDelay(i) }"
        @click="n.url && openUrl(n.url).catch(() => {})"
      >
        <div v-if="n.image" class="news-image">
          <img :src="n.image" :alt="n.image_alt" loading="lazy" />
        </div>
        <div class="news-body">
          <h3 class="news-title">{{ n.title }}</h3>
          <p v-if="n.description" class="news-desc">{{ n.description }}</p>
          <div class="news-meta">
            <span v-if="n.author" class="meta-author">{{ n.author }}</span>
            <span class="meta-date">{{ fmtDate(n.time) }}</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.news-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 18px 24px;
  margin-bottom: 16px;
}
.news-header h1 {
  font-size: 22px;
  font-weight: 700;
  margin: 0;
}
.refresh-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: 1px solid var(--border);
  background: var(--panel);
  color: var(--text-1);
  border-radius: 9px;
  padding: 7px 14px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
  transition: all 0.15s;
}
.refresh-btn:hover:not(:disabled) {
  background: var(--panel-hover);
}
.refresh-btn:disabled {
  opacity: 0.5;
  cursor: default;
}
.btn-icon {
  font-size: 14px;
}
.loading,
.empty {
  padding: 48px 24px;
  text-align: center;
  color: var(--text-3);
  border-radius: 14px;
}
.news-list {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.news-card {
  display: flex;
  border-radius: 14px;
  overflow: hidden;
  cursor: pointer;
  transition: transform 0.15s ease, box-shadow 0.15s ease;
  /* 卡片逐个进场（同首页 pin-card：translateY 28px 上浮 + 淡入，backwards 让
     未出场的卡片先保持隐藏）。延迟由模板内联 animationDelay 提供。 */
  animation: news-card-enter 0.34s cubic-bezier(0.22, 0.61, 0.36, 1) backwards;
}
@keyframes news-card-enter {
  from {
    transform: translateY(28px);
    opacity: 0;
  }
  to {
    transform: none;
    opacity: 1;
  }
}
.news-card:hover {
  transform: translateY(-2px);
  box-shadow: 0 4px 20px var(--k-15);
}
.news-card:active {
  transform: scale(0.99);
}
.news-image {
  flex-shrink: 0;
  width: 200px;
  height: 130px;
  overflow: hidden;
  background: var(--w-03);
}
.news-image img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.news-body {
  flex: 1;
  padding: 14px 16px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}
.news-title {
  font-size: 15px;
  font-weight: 700;
  margin: 0;
  color: var(--text-1);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.news-desc {
  font-size: 12px;
  color: var(--text-3);
  margin: 0;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  line-height: 1.5;
}
.news-meta {
  display: flex;
  gap: 12px;
  font-size: 11px;
  color: var(--text-3);
  margin-top: auto;
}
.meta-author {
  color: var(--accent);
  font-weight: 600;
}

/* 宽屏：单列长条改成多列卡片，充分利用横向空间 */
@media (min-width: 1180px) {
  .news-list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(380px, 1fr));
    gap: 16px;
    align-items: start;
  }
  .news-card {
    flex-direction: column;
  }
  .news-image {
    width: 100%;
    height: 170px;
  }
}

@media (max-width: 640px) {
  .news-card {
    flex-direction: column;
  }
  .news-image {
    width: 100%;
    height: 180px;
  }
}
</style>
