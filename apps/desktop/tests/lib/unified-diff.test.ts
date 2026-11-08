import { describe, expect, test } from 'bun:test'
import { parseUnifiedDiff } from '../../src/lib/unified-diff'

const DIFF = `diff --git a/src/a.ts b/src/a.ts
index 1111111..2222222 100644
--- a/src/a.ts
+++ b/src/a.ts
@@ -10,3 +10,4 @@ export function a() {
 const one = 1
-const two = 2
+const two = 3
+const three = 3
 return one
\\ No newline at end of file
`

describe('parseUnifiedDiff', () => {
  test('numbers lines from the hunk header and drops file headers', () => {
    const lines = parseUnifiedDiff(DIFF).map(
      ({ kind, text, oldLine, newLine }) => [kind, text, oldLine, newLine]
    )

    expect(lines).toEqual([
      ['hunk', '@@ -10,3 +10,4 @@ export function a() {', null, null],
      ['context', 'const one = 1', 10, 10],
      ['removed', 'const two = 2', 11, null],
      ['added', 'const two = 3', null, 11],
      ['added', 'const three = 3', null, 12],
      ['context', 'return one', 12, 13],
      ['note', '\\ No newline at end of file', null, null]
    ])
  })

  test('a new file counts from line one', () => {
    const lines = parseUnifiedDiff(
      '--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+a\n+b\n'
    )

    expect(lines.slice(1).map((line) => line.newLine)).toEqual([1, 2])
  })

  test('an empty diff has no lines', () => {
    expect(parseUnifiedDiff('')).toEqual([])
  })
})
