import { api } from "../api";
import type { CreatedEntry, DirectoryListing, FileDocument, FileInfo, RenamedEntry } from "../types";

export interface FileWorkspaceOperations {
  list: (rel: string) => Promise<DirectoryListing>;
  reveal: (rel: string) => Promise<void>;
  read: (rel: string) => Promise<FileDocument>;
  write: (rel: string, content: string) => Promise<FileInfo>;
  structure?: {
    create: (rel: string, isDir: boolean) => Promise<CreatedEntry>;
    rename: (rel: string, name: string) => Promise<RenamedEntry>;
    remove: (rel: string) => Promise<void>;
  };
}

export function instanceFileOperations(id: string): FileWorkspaceOperations {
  return {
    list: (path) => api.listInstanceDir(id, path),
    reveal: (path) => api.revealInstancePath(id, path),
    read: (path) => api.readInstanceFile(id, path),
    write: (path, content) => api.writeInstanceFile(id, path, content),
    structure: {
      create: (path, isDir) => api.createInstanceEntry(id, path, isDir),
      rename: (path, name) => api.renameInstancePath(id, path, name),
      remove: (path) => api.deleteInstancePath(id, path),
    },
  };
}

export function hostedFileOperations(id: string): FileWorkspaceOperations {
  return {
    list: (path) => api.listHostedServerDir(id, path),
    reveal: (path) => api.revealHostedServerPath(id, path),
    read: (path) => api.readHostedServerFile(id, path),
    write: (path, content) => api.writeHostedServerFile(id, path, content),
  };
}
