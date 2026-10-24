import { computed, ref } from "vue";

/**
 * 悬浮岛侧边栏的共享 UI 状态。
 *
 * 侧边栏自身（SideBar.vue）写入：折叠窄栏时内容区让位（body 加左留白），
 * 展开成浮层时覆盖在内容上方（内容区不再留白）。App.vue 读取该状态
 * 给 .body 加动态 padding；其他需要感知侧边栏显隐的组件也可注入使用。
 */

const revealed = ref(false);

/** 展开（浮层覆盖态）— 内容区占满全宽。 */
export function setSidebarRevealed(v: boolean) {
  revealed.value = v;
}

/** 当前侧边栏是否处于展开态。 */
export function useSidebarRevealed() {
  return computed(() => revealed.value);
}
