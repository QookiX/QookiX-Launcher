import { ref } from "vue";

/**
 * 下载中心右抽屉的共享 UI 状态。
 *
 * 标题栏下载按钮、设置页 / 更新弹窗的「下载并安装」、deep-link 等
 * 多处触发点都通过 openDownloadsDrawer() 打开；DownloadsDrawer 组件
 * 自身读取 open 控制显隐。取代原 /downloads 独立页面。
 */

const open = ref(false);

/** 打开下载抽屉。 */
export function openDownloadsDrawer() {
  open.value = true;
}

/** 关闭下载抽屉。 */
export function closeDownloadsDrawer() {
  open.value = false;
}

/** 抽屉是否打开。 */
export function useDownloadsDrawerOpen() {
  return open;
}
