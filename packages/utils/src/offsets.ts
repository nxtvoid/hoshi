/** Where each of `sizes` starts when laid end to end: `[2, 3, 1]` gives `[0, 2, 5]`. */
export function offsets(sizes: readonly number[]): number[] {
  const starts: number[] = []
  let at = 0
  for (const size of sizes) {
    starts.push(at)
    at += size
  }
  return starts
}
