import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";

export interface RegionStatus {
  /** 是否命中中国大陆地区限制 */
  restricted: boolean;
  region: "mainland_china" | "other";
  /** 判定所用的系统时区名（诊断用） */
  timezone: string;
  /** 命中限制时的提示文案（后端下发，前端直接展示） */
  notice: string;
}

/**
 * 地区可用性。
 *
 * 出于合规原因，中国大陆地区停用「下载并安装 Minecraft 游戏本体」相关功能：
 * 全新实例安装、加载器安装、整合包下载安装、BMCLAPI 镜像下载。
 * 已有实例的启动、管理、模组 / 资源包等内容的下载不受影响。
 *
 * 判定在后端离线完成（读系统时区），前端只读结果，不做二次推断。
 */
export const useRegionStore = defineStore("region", {
  state: () => ({
    status: null as RegionStatus | null,
    loaded: false,
  }),
  getters: {
    /** 游戏下载是否被停用（未加载时按「不限制」处理，避免首屏误灰） */
    downloadBlocked: (s) => !!s.status?.restricted,
    notice: (s) => s.status?.notice ?? "",
  },
  actions: {
    async load() {
      try {
        this.status = await invoke<RegionStatus>("get_region_status");
      } catch {
        // 判定失败时按「不限制」处理：不因为一次命令失败就砍掉功能
        this.status = { restricted: false, region: "other", timezone: "", notice: "" };
      } finally {
        this.loaded = true;
      }
    },
  },
});
