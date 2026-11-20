import { onScopeDispose } from "vue";
import { listen, type Event, type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";

export function createTauriListenerScope() {
  const listeners = new Set<UnlistenFn>();
  let disposed = false;

  async function observe<T>(
    subscribe: (handler: (event: T) => void) => Promise<UnlistenFn>,
    handle: (event: T) => void,
  ): Promise<UnlistenFn> {
    const noop = () => {};
    if (disposed) return noop;
    let active = true;
    let unlisten: UnlistenFn;
    try {
      unlisten = await subscribe((event) => {
        if (!disposed && active) handle(event);
      });
    } catch (error) {
      if (disposed) return noop;
      throw error;
    }
    const detach = () => {
      if (!active) return;
      active = false;
      listeners.delete(detach);
      unlisten();
    };
    if (disposed) detach();
    else listeners.add(detach);
    return detach;
  }

  function bind<T>(event: string, handle: EventCallback<T>) {
    return observe<Event<T>>((callback) => listen(event, callback), handle);
  }

  function dispose() {
    if (disposed) return;
    disposed = true;
    for (const detach of listeners) detach();
  }

  return { listen: bind, observe, dispose };
}

export function useTauriListenerScope() {
  const scope = createTauriListenerScope();
  onScopeDispose(scope.dispose);
  return scope;
}
