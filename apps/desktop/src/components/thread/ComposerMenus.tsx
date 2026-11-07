import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import type { Effort } from '@/bindings/Effort'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import {
  ArrowDown01Icon,
  ComputerIcon,
  GitForkIcon
} from '@hugeicons/core-free-icons'
import {
  CLAUDE_MODELS,
  type DraftWorkspace,
  EFFORTS,
  useComposerStore
} from '@/state/composer-store'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'

/** Radio values can't be null, so "use the provider default" is this. */
const DEFAULT = 'default'

const EFFORT_LABELS: Record<Effort, () => string> = {
  low: m.effort_low,
  medium: m.effort_medium,
  high: m.effort_high,
  xhigh: m.effort_xhigh,
  max: m.effort_max
}

export function ModelMenu() {
  const thread = useAppStore((state) =>
    state.threads.find((candidate) => candidate.id === state.selectedThreadId)
  )
  const draftModel = useComposerStore((state) => state.model)
  const draftEffort = useComposerStore((state) => state.effort)
  const setModel = useAppStore((state) => state.setModel)
  const model = thread ? thread.model : draftModel
  const effort = thread ? thread.effort : draftEffort
  const modelLabel = CLAUDE_MODELS.find(
    (option) => option.value === model
  )?.label

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={<Button variant='ghost' size='xs' className='-mr-1.5' />}
      >
        <span className='text-foreground font-medium'>{m.claude_code()}</span>
        {modelLabel ? <span>{modelLabel}</span> : null}
        {effort ? <span>{EFFORT_LABELS[effort]()}</span> : null}
        <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align='end' side='top' className='w-44'>
        <DropdownMenuGroup>
          <DropdownMenuLabel>{m.model_label()}</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={model ?? DEFAULT}
            onValueChange={(value: string) =>
              void setModel(value === DEFAULT ? null : value, effort)
            }
          >
            <DropdownMenuRadioItem value={DEFAULT}>
              {m.provider_default()}
            </DropdownMenuRadioItem>
            {CLAUDE_MODELS.map(({ value, label }) => (
              <DropdownMenuRadioItem key={value} value={value}>
                {label}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuLabel>{m.effort_label()}</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={effort ?? DEFAULT}
            onValueChange={(value: string) =>
              void setModel(model, value === DEFAULT ? null : (value as Effort))
            }
          >
            <DropdownMenuRadioItem value={DEFAULT}>
              {m.provider_default()}
            </DropdownMenuRadioItem>
            {EFFORTS.map((value) => (
              <DropdownMenuRadioItem key={value} value={value}>
                {EFFORT_LABELS[value]()}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

const WORKSPACES = [
  { value: 'local', label: m.workspace_local, icon: ComputerIcon },
  { value: 'newWorktree', label: m.workspace_new_worktree, icon: GitForkIcon }
] as const

/** Where a new thread will work; fixed once the thread exists. */
export function WorkspaceMenu() {
  const workspace = useComposerStore((state) => state.workspace)
  const setWorkspace = useComposerStore((state) => state.setWorkspace)
  const current =
    WORKSPACES.find((option) => option.value === workspace) ?? WORKSPACES[0]

  return (
    <DropdownMenu>
      <DropdownMenuTrigger render={<Button variant='ghost' size='xs' />}>
        <HugeiconsIcon icon={current.icon} strokeWidth={2} />
        {current.label()}
        <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align='end' side='top' className='w-44'>
        <DropdownMenuGroup>
          <DropdownMenuLabel>{m.workspace_label()}</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={workspace}
            onValueChange={(value: DraftWorkspace) => setWorkspace(value)}
          >
            {WORKSPACES.map(({ value, label, icon }) => (
              <DropdownMenuRadioItem key={value} value={value}>
                <HugeiconsIcon icon={icon} strokeWidth={2} />
                {label()}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
