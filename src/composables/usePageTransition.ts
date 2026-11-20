import { ref } from "vue";
import type { RouteMeta } from "vue-router";

// 根据侧边栏导航项顺序计算页面转场方向。
// 旧页面用 transform 移出提供方向感（淡出中磨砂异常不可见），
// 新页面纯 opacity 淡入（无 transform，backdrop-filter 全程稳定）。
export const transitionName = ref("page-fade");

export const navOrder = {
  home: 0,
  browse: 1,
  instances: 2,
  multiplayer: 3,
  skins: 4,
  toolbox: 5,
  timemachine: 6,
  settings: 7,
} as const;

declare module "vue-router" {
  interface RouteMeta {
    navSection?: keyof typeof navOrder;
    detail?: boolean;
  }
}

export function setTransitionName(from: RouteMeta, to: RouteMeta) {
  const sectionDelta = from.navSection && to.navSection
    ? navOrder[to.navSection] - navOrder[from.navSection]
    : 0;
  const direction = sectionDelta || Number(!!to.detail) - Number(!!from.detail);
  transitionName.value = direction > 0 ? "slide-forward" : direction < 0 ? "slide-back" : "page-fade";
}
