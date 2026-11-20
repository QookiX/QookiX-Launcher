import type { BlockState, MeshResult } from "./schematicMeshWorker";

interface PendingTask {
  chunkPos: [number, number, number];
  blocks: Uint32Array;
  neighbors?: Uint32Array | null;
  resolve: (value: MeshResult) => void;
  reject: (reason: unknown) => void;
}

const INITIALIZATION_TIMEOUT_MS = 30_000;
const MESH_TIMEOUT_MS = 30_000;

export interface SchematicWorkerPool {
  init: (resourceBase?: string) => Promise<void>;
  /** 调色板一次性下发（`blocks` 里存的是它的下标） */
  setPalette: (palette: BlockState[]) => void;
  buildMesh: (
    chunkPos: [number, number, number],
    blocks: Uint32Array,
    neighbors?: Uint32Array | null,
  ) => Promise<MeshResult>;
  dispose: () => void;
}

export function createSchematicWorkerPool(size: number): SchematicWorkerPool {
  const workers: Worker[] = [];
  const ready = new Set<Worker>();
  const idle: Worker[] = [];
  const running = new Map<Worker, { task: PendingTask; timeout: ReturnType<typeof setTimeout> }>();
  const queue: PendingTask[] = [];
  let initialized: Promise<void> | undefined;
  let initialization: { resolve: () => void; reject: (error: Error) => void } | undefined;
  let initializationTimeout: ReturnType<typeof setTimeout> | undefined;
  let closed: Error | undefined;
  const maxQueued = 24;

  function close(error: Error) {
    if (closed) return;
    closed = error;
    clearTimeout(initializationTimeout);
    initialization?.reject(error);
    initialization = undefined;
    for (const { task, timeout } of running.values()) {
      clearTimeout(timeout);
      task.reject(error);
    }
    for (const task of queue) task.reject(error);
    for (const worker of workers) worker.terminate();
    workers.length = 0;
    idle.length = 0;
    queue.length = 0;
    ready.clear();
    running.clear();
  }

  function handleWorkerMessage(worker: Worker, event: MessageEvent) {
    if (closed) return;
    const data = event.data;
    if (data.type === "error") {
      close(new Error(data.message || "Schematic worker failed"));
      return;
    }
    if (data.type === "ready") {
      if (ready.has(worker)) return;
      ready.add(worker);
      dispatch(worker);
      if (ready.size === size) {
        clearTimeout(initializationTimeout);
        initialization?.resolve();
        initialization = undefined;
      }
      return;
    }
    if (data.type === "meshResult") {
      const active = running.get(worker);
      if (!active) return;
      running.delete(worker);
      clearTimeout(active.timeout);
      active.task.resolve({ opaque: data.opaque, translucent: data.translucent });
      dispatch(worker);
    }
  }

  function dispatch(worker: Worker) {
    const task = queue.shift();
    if (task) sendToWorker(worker, task);
    else idle.push(worker);
  }

  function sendToWorker(worker: Worker, task: PendingTask) {
    const timeout = setTimeout(() => close(new Error("Schematic mesh worker timed out")), MESH_TIMEOUT_MS);
    running.set(worker, { task, timeout });
    // 只传 16 KiB 的方块数据（+ 邻块边界）；调色板每个 worker 只收一次
    // （否则每块重传一份，一份一千多个对象，几千个区块下来光复制就是几十万次对象拷贝）
    const transfer = [task.blocks.buffer];
    if (task.neighbors && task.neighbors.buffer !== task.blocks.buffer) transfer.push(task.neighbors.buffer);
    try {
      worker.postMessage(
        {
          type: "mesh",
          chunkPos: task.chunkPos,
          blocks: task.blocks,
          neighbors: task.neighbors ?? null,
        },
        transfer,
      );
    } catch (error) {
      close(error instanceof Error ? error : new Error(String(error)));
    }
  }

  function setPalette(palette: BlockState[]) {
    if (closed) throw closed;
    for (const worker of workers) {
      try {
        worker.postMessage({ type: "palette", palette });
      } catch (error) {
        close(error instanceof Error ? error : new Error(String(error)));
        throw error;
      }
    }
  }

  function init(resourceBase?: string): Promise<void> {
    if (closed) return Promise.reject(closed);
    if (initialized) return initialized;
    initialized = new Promise((resolve, reject) => {
      initialization = { resolve, reject };
      initializationTimeout = setTimeout(
        () => close(new Error("Schematic worker initialization timed out")),
        INITIALIZATION_TIMEOUT_MS,
      );
      try {
        if (!Number.isInteger(size) || size < 1) throw new Error("Schematic worker count must be positive");
        for (let i = 0; i < size; i++) {
          const worker = new Worker(new URL("./schematicMeshWorker.ts", import.meta.url), { type: "module" });
          workers.push(worker);
          worker.onmessage = (event) => handleWorkerMessage(worker, event);
          worker.onerror = (event) => close(new Error(event.message || "Schematic worker failed"));
          worker.onmessageerror = () => close(new Error("Invalid schematic worker response"));
          worker.postMessage({ type: "init", resourceBase });
        }
      } catch (error) {
        close(error instanceof Error ? error : new Error(String(error)));
      }
    });
    return initialized;
  }

  function buildMesh(
    chunkPos: [number, number, number],
    blocks: Uint32Array,
    neighbors?: Uint32Array | null,
  ): Promise<MeshResult> {
    if (closed) return Promise.reject(closed);
    if (!initialized) return Promise.reject(new Error("Schematic worker pool is not initialized"));
    if (!idle.length && queue.length >= maxQueued) {
      return Promise.reject(new Error("Schematic mesh queue is full"));
    }
    return new Promise((resolve, reject) => {
      const task: PendingTask = { chunkPos, blocks, neighbors, resolve, reject };
      const worker = idle.pop();
      if (worker) {
        sendToWorker(worker, task);
      } else {
        queue.push(task);
      }
    });
  }

  function dispose() {
    close(new DOMException("Schematic preview closed", "AbortError"));
  }

  return { init, setPalette, buildMesh, dispose };
}
