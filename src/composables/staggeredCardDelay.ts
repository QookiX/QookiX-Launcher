export function staggeredCardDelay(index: number, count: number): string {
  const step = Math.min(0.045, 0.6 / Math.max(1, count));
  return `${(0.06 + index * step).toFixed(3)}s`;
}
