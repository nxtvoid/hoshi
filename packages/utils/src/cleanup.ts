/**
 * Runs `work`, then `cleanup` however it ended. React Compiler can't compile a component
 * holding a `try`/`finally`, so components put theirs here.
 */
export async function withCleanup<T>(
  work: () => Promise<T>,
  cleanup: () => void
): Promise<T> {
  try {
    return await work()
  } finally {
    cleanup()
  }
}
