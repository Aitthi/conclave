// Local event bus standing in for Tauri's event system in fixture mode.
// v1 scenarios never emit; this exists so useEvent() subscriptions are silent
// no-ops instead of DEV console errors, and so a later scenario CAN emit.
const bus = new EventTarget();

export function fixtureListen<T>(event: string, cb: (payload: T) => void): () => void {
  const h = (e: Event) => cb((e as CustomEvent<T>).detail);
  bus.addEventListener(event, h);
  return () => bus.removeEventListener(event, h);
}

export function emitFixtureEvent<T>(event: string, payload: T): void {
  bus.dispatchEvent(new CustomEvent(event, { detail: payload }));
}
