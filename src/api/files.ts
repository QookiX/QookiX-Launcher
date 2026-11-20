import { invoke } from "./invoke";
import type { CreatedEntry, DirectoryListing, FileDocument, FileInfo, RenamedEntry } from "../types";

export const fileApi = {
  // instance file manager
  listInstanceDir: (instanceId: string, rel: string) =>
    invoke<DirectoryListing>("list_instance_dir", { instanceId, rel }),
  readInstanceFile: (instanceId: string, rel: string) =>
    invoke<FileDocument>(
      "read_instance_file",
      { instanceId, rel }
    ),
  writeInstanceFile: (instanceId: string, rel: string, content: string) =>
    invoke<FileInfo>(
      "write_instance_file",
      { instanceId, rel, content }
    ),
  createInstanceEntry: (instanceId: string, rel: string, isDir: boolean) =>
    invoke<CreatedEntry>("create_instance_entry", { instanceId, rel, isDir }),
  deleteInstancePath: (instanceId: string, rel: string) =>
    invoke<void>("delete_instance_path", { instanceId, rel }),
  renameInstancePath: (instanceId: string, rel: string, newName: string) =>
    invoke<RenamedEntry>("rename_instance_path", { instanceId, rel, newName }),
  revealInstancePath: (instanceId: string, rel: string) =>
    invoke<void>("reveal_instance_path", { instanceId, rel }),
  listHostedServerDir: (id: string, rel: string) =>
    invoke<DirectoryListing>("list_hosted_server_dir", { id, rel }),
  revealHostedServerPath: (id: string, rel: string) =>
    invoke<void>("reveal_hosted_server_path", { id, rel }),
  readHostedServerFile: (id: string, rel: string) =>
    invoke<FileDocument>(
      "read_hosted_server_file",
      { id, rel }
    ),
  writeHostedServerFile: (id: string, rel: string, content: string) =>
    invoke<FileInfo>(
      "write_hosted_server_file",
      { id, rel, content }
    ),
};
