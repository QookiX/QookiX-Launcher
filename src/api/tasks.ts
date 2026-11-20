import { invoke } from "./invoke";
import type { InstallPlan, LaunchResult } from "../types";

export const taskApi = {
  installGame: (instanceId: string) =>
    invoke<InstallPlan>("install_game", { instanceId }),
  cancelInstall: (instanceId?: string) => invoke<void>("cancel_install", { instanceId: instanceId ?? null }),
  cancelInstallTask: (taskId: number) => invoke<void>("cancel_install", { taskId, instanceId: null }),
  launchInstance: (instanceId: string, world?: string, server?: string) =>
    invoke<LaunchResult>(
      "launch_instance",
      {
        instanceId,
        world: world ?? null,
        server: server ?? null,
      }
    ),
  stopGame: () => invoke<void>("stop_game"),
  isGameRunning: () => invoke<boolean>("is_game_running"),
};
