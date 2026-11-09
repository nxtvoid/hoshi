import type { GitAction } from '@/bindings/GitAction'
import type { GitStatus } from '@/bindings/GitStatus'

/** What the main button is named for. */
export type GitActionLabel =
  | 'commit'
  | 'commitPush'
  | 'commitPushPullRequest'
  | 'push'
  | 'pushCreatePullRequest'
  | 'createPullRequest'
  | 'pull'

/** Why an action can't run now. */
export type GitBlocker =
  | 'detached'
  | 'clean'
  | 'uncommitted'
  | 'behind'
  | 'diverged'
  | 'noRemote'
  | 'noCommits'
  | 'upToDate'

export type QuickAction = {
  label: GitActionLabel
  action: GitAction
  /** Set when the button shows, but can't run. */
  blocker: GitBlocker | null
}

export function onDefaultBranch(status: GitStatus) {
  return status.branch !== null && status.branch === status.defaultBranch
}

/**
 * The action the main button runs, as t3code picks it: commit what changed and take it as
 * far as it can go, push what's ahead, pull what's behind, or nothing when up to date.
 */
export function quickAction(
  status: GitStatus,
  pullRequestOpen: boolean
): QuickAction {
  const blocked = (blocker: GitBlocker): QuickAction => ({
    label: 'commit',
    action: 'commit',
    blocker
  })
  if (status.branch === null) return blocked('detached')
  const settled = pullRequestOpen || onDefaultBranch(status)
  if (status.hasChanges) {
    if (status.behind > 0 || (!status.hasUpstream && !status.hasRemote)) {
      return { label: 'commit', action: 'commit', blocker: null }
    }
    return settled
      ? { label: 'commitPush', action: 'commitPush', blocker: null }
      : {
          label: 'commitPushPullRequest',
          action: 'commitPushPullRequest',
          blocker: null
        }
  }
  if (!status.hasUpstream) {
    if (!status.hasRemote) return blocked('noRemote')
    if (status.aheadOfDefault === 0) {
      return {
        label: 'push',
        action: 'push',
        blocker: pullRequestOpen ? 'upToDate' : 'noCommits'
      }
    }
    return settled
      ? { label: 'push', action: 'push', blocker: null }
      : {
          label: 'pushCreatePullRequest',
          action: 'createPullRequest',
          blocker: null
        }
  }
  if (status.ahead > 0 && status.behind > 0) {
    return { label: 'pull', action: 'pull', blocker: 'diverged' }
  }
  if (status.behind > 0) return { label: 'pull', action: 'pull', blocker: null }
  if (status.ahead > 0) {
    return settled
      ? { label: 'push', action: 'push', blocker: null }
      : {
          label: 'pushCreatePullRequest',
          action: 'createPullRequest',
          blocker: null
        }
  }
  if (
    !pullRequestOpen &&
    status.aheadOfDefault > 0 &&
    !onDefaultBranch(status)
  ) {
    return {
      label: 'createPullRequest',
      action: 'createPullRequest',
      blocker: null
    }
  }
  return blocked('upToDate')
}

/** Why each menu entry can't run now; `null` when it can. */
export function menuBlockers(status: GitStatus) {
  const pushBlocker = (): GitBlocker | null => {
    if (status.branch === null) return 'detached'
    if (status.hasChanges) return 'uncommitted'
    if (status.behind > 0) return 'behind'
    if (!status.hasRemote) return 'noRemote'
    const ahead = status.hasUpstream ? status.ahead : status.aheadOfDefault
    return ahead === 0 ? 'noCommits' : null
  }
  const pullRequestBlocker = (): GitBlocker | null => {
    if (status.branch === null) return 'detached'
    if (status.behind > 0) return 'behind'
    if (!status.hasRemote) return 'noRemote'
    return status.aheadOfDefault === 0 ? 'noCommits' : null
  }
  return {
    commit: status.hasChanges ? null : ('clean' as GitBlocker),
    push: pushBlocker(),
    createPullRequest: pullRequestBlocker()
  }
}

/** Pushing straight to the default branch asks first, as it skips review. */
export function pushesToDefaultBranch(status: GitStatus, action: GitAction) {
  return action !== 'commit' && action !== 'pull' && onDefaultBranch(status)
}
