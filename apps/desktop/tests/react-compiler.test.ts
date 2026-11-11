import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, test } from 'bun:test'
import { transform } from 'oxc-transform-react'

const source = join(import.meta.dir, '../src')

const files = readdirSync(source, { recursive: true })
  .map(String)
  .filter((file) => /\.tsx?$/.test(file) && !file.endsWith('.gen.ts'))
  .filter((file) => !file.replaceAll('\\', '/').startsWith('paraglide/'))

/**
 * React Compiler skips what it can't prove safe, and a skipped file still builds, only
 * without its memoization; this keeps every file within what it compiles.
 */
describe('React Compiler', () => {
  test.each(files)('compiles %s', async (file) => {
    const path = join(source, file)
    const code = readFileSync(path, 'utf8')
    if (code.includes("'use no memo'")) return
    const result = await transform(path, code, {
      jsx: { runtime: 'automatic', development: false, refresh: false },
      reactCompiler: { reportDiagnostics: true }
    })
    expect(
      result.errors.map((error) => error.codeframe ?? error.message)
    ).toEqual([])
  })
})
