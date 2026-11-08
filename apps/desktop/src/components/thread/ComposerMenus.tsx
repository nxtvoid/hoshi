import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { useSettingsStore } from '@/state/settings-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import type { AccessMode } from '@/bindings/AccessMode'
import {
  type AccessChoice,
  type DraftWorkspace,
  useComposerStore
} from '@/state/composer-store'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
import {
  AiMagicIcon,
  ArrowDown01Icon,
  ComputerIcon,
  GitForkIcon,
  PencilEdit02Icon,
  ShieldUserIcon,
  SquareUnlock02Icon,
  TaskEdit01Icon
} from '@hugeicons/core-free-icons'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'

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

const ACCESS_MODES = [
  {
    value: 'ask',
    label: m.access_ask,
    description: m.access_ask_description,
    icon: ShieldUserIcon
  },
  {
    value: 'acceptEdits',
    label: m.access_accept_edits,
    description: m.access_accept_edits_description,
    icon: PencilEdit02Icon
  },
  {
    value: 'auto',
    label: m.access_auto,
    description: m.access_auto_description,
    icon: AiMagicIcon
  },
  {
    value: 'fullAccess',
    label: m.access_full,
    description: m.access_full_description,
    icon: SquareUnlock02Icon
  }
] as const

export function useAccess(): [
  AccessChoice,
  (change: Partial<AccessChoice>) => void
] {
  const thread = useAppStore((state) =>
    state.threads.find((candidate) => candidate.id === state.selectedThreadId)
  )
  const draftAccessMode = useComposerStore((state) => state.accessMode)
  const draftPlanMode = useComposerStore((state) => state.planMode)
  const setAccess = useAppStore((state) => state.setAccess)
  const choice: AccessChoice = thread
    ? { accessMode: thread.accessMode, planMode: thread.planMode }
    : { accessMode: draftAccessMode, planMode: draftPlanMode }
  return [choice, (change) => void setAccess({ ...choice, ...change })]
}

/** What the agent may do without asking, from the next turn on. */
export function AccessMenu() {
  const [choice, update] = useAccess()
  const current =
    ACCESS_MODES.find((option) => option.value === choice.accessMode) ??
    ACCESS_MODES[1]

  return (
    <DropdownMenu>
      <DropdownMenuTrigger render={<Button variant='ghost' size='xs' />}>
        <HugeiconsIcon icon={current.icon} strokeWidth={2} />
        {current.label()}
        <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align='end' side='top' className='w-72'>
        <DropdownMenuGroup>
          <DropdownMenuLabel>{m.access_label()}</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={choice.accessMode}
            onValueChange={(accessMode: AccessMode) => update({ accessMode })}
          >
            {ACCESS_MODES.map(({ value, label, description, icon }) => (
              <DropdownMenuRadioItem
                key={value}
                value={value}
                className='items-start py-1.5'
              >
                <HugeiconsIcon icon={icon} strokeWidth={2} className='mt-0.5' />
                <span className='flex min-w-0 flex-col'>
                  {label()}
                  <span className='text-muted-foreground text-xs'>
                    {description()}
                  </span>
                </span>
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** Hidden unless turned on in settings, but kept while a thread is still planning. */
export function PlanToggle() {
  const [choice, update] = useAccess()
  const enabled = useSettingsStore(
    (state) => state.settings?.planModeEnabled ?? false
  )
  if (!enabled && !choice.planMode) return null

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant='ghost'
            size='xs'
            aria-pressed={choice.planMode}
            onClick={() => update({ planMode: !choice.planMode })}
            className={cn(choice.planMode && 'bg-accent text-foreground')}
          />
        }
      >
        <HugeiconsIcon icon={TaskEdit01Icon} strokeWidth={2} />
        {m.plan_mode()}
      </TooltipTrigger>
      <TooltipContent>{m.plan_mode_description()}</TooltipContent>
    </Tooltip>
  )
}
