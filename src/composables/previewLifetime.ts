export interface PreviewSession {
  id: string;
  signal: AbortSignal;
}

/** Owns the current backend session and closes results that arrive after cancellation. */
export function createPreviewLifetime(closeSession: (id: string) => Promise<unknown>) {
  let opening: AbortController | undefined;
  let active: { id: string; controller: AbortController } | undefined;
  let disposed = false;
  let pendingOpen: Promise<unknown> = Promise.resolve();

  function release(id: string) {
    void closeSession(id).catch((error) => console.warn("Failed to close preview session:", error));
  }

  function beginOpen() {
    opening?.abort();
    opening = new AbortController();
    if (disposed) opening.abort();
    return opening.signal;
  }

  function close(id: string) {
    if (active?.id !== id) return;
    active.controller.abort();
    active = undefined;
    release(id);
  }

  function activate(request: AbortSignal, id: string): PreviewSession | null {
    if (request.aborted || request !== opening?.signal) {
      release(id);
      return null;
    }
    if (active) close(active.id);
    active = { id, controller: new AbortController() };
    return { id, signal: active.controller.signal };
  }

  function open<T extends { sessionId: string }>(request: AbortSignal, load: () => Promise<T>) {
    const result = pendingOpen.then(async () => {
      if (request.aborted) return null;
      const manifest = await load();
      const session = activate(request, manifest.sessionId);
      return session ? { manifest, session } : null;
    });
    pendingOpen = result.catch(() => {});
    return result;
  }

  function dispose() {
    disposed = true;
    opening?.abort();
    if (active) close(active.id);
  }

  return { beginOpen, open, close, dispose };
}
