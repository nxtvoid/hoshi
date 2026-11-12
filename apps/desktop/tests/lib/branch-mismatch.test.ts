import { describe, expect, test } from 'bun:test'
import { branchMismatch, mismatchKey } from '@/lib/branch-mismatch'

const local = { branch: 'fix-login', worktreePath: null, projectId: 'p' }

describe('branchMismatch', () => {
  test('a local thread on another branch than the root mismatches', () => {
    expect(branchMismatch(local, 'main')).toEqual({
      threadBranch: 'fix-login',
      rootBranch: 'main'
    })
  })

  test('a local thread on the root branch matches', () => {
    expect(branchMismatch(local, 'fix-login')).toBeNull()
  })

  test('a worktree thread never mismatches', () => {
    expect(
      branchMismatch({ ...local, worktreePath: '/w/fix-login' }, 'main')
    ).toBeNull()
  })

  test('nothing is known without both branches or a project', () => {
    expect(branchMismatch(local, null)).toBeNull()
    expect(branchMismatch(local, undefined)).toBeNull()
    expect(branchMismatch({ ...local, branch: null }, 'main')).toBeNull()
    expect(branchMismatch({ ...local, projectId: null }, 'main')).toBeNull()
  })

  test('each pair of branches has its own key', () => {
    const first = mismatchKey('t', { threadBranch: 'a', rootBranch: 'b' })
    const next = mismatchKey('t', { threadBranch: 'a', rootBranch: 'c' })
    expect(first).not.toBe(next)
  })
})
