/**
 * 系统通知：只在启动器不在前台时弹（前台已经有角标和面板，不必打扰）。
 *
 * 用 `document.hasFocus()` 判断焦点，省掉 `core:window:allow-is-focused` 权限；
 * 通知本身走 tauri-plugin-notification（权限见 capabilities/default.json）。
 */
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

let granted: boolean | null = null;

async function ensurePermission(): Promise<boolean> {
  if (granted !== null) return granted;
  try {
    granted = (await isPermissionGranted()) || (await requestPermission()) === "granted";
  } catch {
    granted = false;
  }
  return granted;
}

export async function notifyOs(title: string, body: string) {
  try {
    if (document.hasFocus()) return;
    if (!(await ensurePermission())) return;
    sendNotification({ title, body });
  } catch {
    /* 通知失败（系统关了通知 / 插件不可用）不该影响主流程 */
  }
}
