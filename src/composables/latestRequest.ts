/** A request may publish its result only while it owns the current generation. */
export function createLatestRequest() {
  let generation = 0;

  return {
    start() {
      const request = ++generation;
      return () => request === generation;
    },
    invalidate() {
      generation++;
    },
  };
}
