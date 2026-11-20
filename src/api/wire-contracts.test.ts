import fixtures from "./wire-contracts.json";
import type { DownloadProgressEvent, FileDocument, InstallProgressEvent, LaunchStateEvent, QookixStatus } from "../types";

// Rust checks the same serialized examples against wire-contracts.json.
const state = fixtures.launch.state;
if (state !== "running" && state !== "exited") throw new Error("Invalid launch state fixture");

export const ipcWireExamples = {
  ...fixtures,
  launch: { ...fixtures.launch, state },
} satisfies {
  file: FileDocument;
  session: QookixStatus;
  install: InstallProgressEvent;
  download: DownloadProgressEvent;
  launch: LaunchStateEvent;
};
