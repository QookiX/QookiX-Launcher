import { invoke as rawInvoke } from "@tauri-apps/api/core";
import { trackStart, trackEnd, trackError } from "../loadingBar";

/**
 * 包装 tauri invoke：只有**网络请求**（显式传 { net: true }）才触发顶部加载条。
 * 本地命令（读写文件、保存设置、列表查询等）默认静默，避免加载条随交互频繁闪烁。
 * 下载/安装类长任务由页面内进度 UI 负责，即使走网络也不传 net。
 * 顶部加载条的另一个出现时机是「页面加载」（路由切换，见 router.ts）。
 */
export function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
  opts?: { net?: boolean }
): Promise<T> {
  if (!opts?.net) return rawInvoke<T>(cmd, args);
  trackStart();
  return rawInvoke<T>(cmd, args).then(
    (res) => {
      trackEnd();
      return res;
    },
    (err) => {
      trackError();
      trackEnd();
      throw err;
    },
  );
}
