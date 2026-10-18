import { ref } from "vue";

/**
 * 实例详情页挂载期间为 true。
 * 让全局 ModpackDropOverlay 不抢占 .zip（交给实例详情页当光影/材质包处理），
 * .mrpack 仍由 ModpackDropOverlay 处理（整合包创建新实例）。
 */
export const instanceDetailActive = ref(false);
