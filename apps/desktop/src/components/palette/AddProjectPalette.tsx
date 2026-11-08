import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { Kbd } from '@hoshi/ui/components/kbd'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { homeDir, sep } from '@tauri-apps/api/path'
import { open } from '@tauri-apps/plugin-dialog'
import { Button } from '@hoshi/ui/components/button'
import type { ForgeKind } from '@/bindings/ForgeKind'
import { isMac } from '@/components/brand/EditorIcon'
import { useSidebarStore } from '@/state/sidebar-store'
import { useNavigate } from '@tanstack/react-router'
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react'
import { ForgeIcon } from '@/components/brand/ForgeIcon'
import type { CommandError } from '@/bindings/CommandError'
import type { FolderListing } from '@/bindings/FolderListing'
import type { RemoteRepository } from '@/bindings/RemoteRepository'
import { loadedSettings, useSettingsStore } from '@/state/settings-store'
import { forgeErrorMessage } from '@/components/panel/pull-request/use-pull-request-actions'
import { forgeLogin, useForgeAccountsStore } from '@/state/forge-accounts-store'
import {
  PALETTE_ICON,
  type PaletteEntry,
  type PaletteGroup,
  PaletteFrame
} from './PaletteFrame'
import {
  ArrowTurnBackwardIcon,
  Folder01Icon,
  FolderAddIcon,
  FolderGitIcon,
  Link01Icon
} from '@hugeicons/core-free-icons'
import {
  completeFolderName,
  endsWithSeparator,
  expandHome,
  filterFolders,
  isAbsolutePath,
  isValidFolderName,
  joinPath,
  repositoryFolderName,
  resolveFolderQuery,
  splitFolderQuery,
  withTrailingSeparator
} from '@/lib/folder-path'

const LIST_DELAY_MS = 60
/** More folders than this wait for the name to narrow them. */
const FOLDER_LIMIT = 200

type CloneSource = 'url' | ForgeKind

type Step =
  | { kind: 'sources' }
  | { kind: 'folder' }
  | { kind: 'newProject' }
  | { kind: 'repository'; source: CloneSource }
  | {
      kind: 'destination'
      source: CloneSource
      url: string
      title: string
      subtitle: string | null
    }

type Frame = { step: Step; query: string }

const INITIAL_FRAME: Frame = { step: { kind: 'sources' }, query: '' }

const FORGE_SOURCES: {
  forge: ForgeKind
  title: () => string
  description: () => string
  placeholder: () => string
}[] = [
  {
    forge: 'github',
    title: m.add_project_github,
    description: m.add_project_github_description,
    placeholder: m.add_project_github_placeholder
  },
  {
    forge: 'bitbucket',
    title: m.add_project_bitbucket,
    description: m.add_project_bitbucket_description,
    placeholder: m.add_project_bitbucket_placeholder
  }
]

function errorMessage(error: CommandError): string {
  switch (error.code) {
    case 'folderUnavailable':
    case 'workspaceRootNotFound':
      return m.error_folder_unavailable({ path: error.path })
    case 'folderNotEmpty':
      return m.error_folder_not_empty({ path: error.path })
    case 'repositoryNameInvalid':
      return m.error_repository_name_invalid()
    case 'forgeUnavailable':
      return m.error_repository_not_found()
    case 'gitFailed':
      return m.error_git_failed({ detail: error.detail })
    case 'forgeUnauthorized':
    case 'forgeRateLimited':
    case 'forgeRejected':
    case 'forgeUnreachable':
      return forgeErrorMessage(error)
    default:
      return m.error_generic()
  }
}

/** Where new projects and clones go: the folder set in settings, or the home folder. */
async function rootFolder() {
  const folder = (await loadedSettings())?.general.projectsFolder?.trim()
  return folder || (await homeDir())
}

function fileManagerLabel(separator: string) {
  if (isMac()) return m.add_project_open_in_finder()
  if (separator === '\\') return m.add_project_open_in_explorer()
  return m.add_project_open_in_file_manager()
}

/** Folder listings, read once per folder while the palette stays open. */
function useFolderListings() {
  const cache = useRef(new Map<string, FolderListing | null>())
  const [, setVersion] = useState(0)

  const fetch = useCallback(async (path: string) => {
    if (cache.current.has(path)) return cache.current.get(path) ?? null
    const listing = await invoke<FolderListing>('list_folders', { path }).catch(
      () => null
    )
    cache.current.set(path, listing)
    setVersion((version) => version + 1)
    return listing
  }, [])

  const read = (path: string | null) => {
    if (!path) return { listing: null, loading: false }
    const found = cache.current.get(path)
    return { listing: found ?? null, loading: found === undefined }
  }

  return { fetch, read }
}

/** Every way to add a project: a folder on disk, a new repository, or a clone. */
export function AddProjectPalette({
  onClose,
  onBack
}: {
  onClose: () => void
  onBack: () => void
}) {
  const addProjectAt = useAppStore((state) => state.addProjectAt)
  const setView = useSidebarStore((state) => state.setView)
  const accounts = useForgeAccountsStore((state) => state.accounts)
  const loadAccounts = useForgeAccountsStore((state) => state.load)
  const projectsFolder = useSettingsStore(
    (state) => state.settings?.general.projectsFolder
  )
  const navigate = useNavigate()
  const separator = sep()
  const [stack, setStack] = useState<Frame[]>([INITIAL_FRAME])
  const [home, setHome] = useState<string | null>(null)
  const [root, setRoot] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [pending, setPending] = useState(false)
  const [activeKey, setActiveKey] = useState<string | undefined>()
  const { fetch: fetchListing, read: readListing } = useFolderListings()

  useEffect(() => {
    void loadAccounts()
    void homeDir().then(setHome)
  }, [loadAccounts])

  useEffect(() => {
    void rootFolder().then(setRoot)
  }, [projectsFolder])

  const { step, query } = stack[stack.length - 1] ?? INITIAL_FRAME
  const browsing = step.kind === 'folder' || step.kind === 'destination'
  const { directory, name } = splitFolderQuery(query, separator)
  const listedDirectory =
    browsing && isAbsolutePath(directory, separator) ? directory : null
  const { listing, loading } = readListing(listedDirectory)

  useEffect(() => {
    if (!listedDirectory) return
    const timer = setTimeout(
      () => void fetchListing(listedDirectory),
      LIST_DELAY_MS
    )
    return () => clearTimeout(timer)
  }, [listedDirectory, fetchListing])

  const setQuery = (next: string) => {
    setError(null)
    setStack((current) => [
      ...current.slice(0, -1),
      { step, query: browsing ? expandHome(next, home, separator) : next }
    ])
  }

  const push = (next: Step, nextQuery: string) => {
    setError(null)
    setStack((current) => [...current, { step: next, query: nextQuery }])
  }

  const back = () => {
    if (pending) return
    setError(null)
    if (stack.length > 1) setStack((current) => current.slice(0, -1))
    else onBack()
  }

  /** Reads the folder first, so the list changes once. */
  const browseTo = async (nextQuery: string) => {
    const target = splitFolderQuery(nextQuery, separator).directory
    if (isAbsolutePath(target, separator)) await fetchListing(target)
    setQuery(nextQuery)
    return false
  }

  const finish = async (path: string) => {
    const failure = await addProjectAt(path)
    if (failure) {
      setError(errorMessage(failure))
      return
    }
    setView('project')
    onClose()
  }

  const attempt = async (work: () => Promise<void>) => {
    if (pending) return
    setPending(true)
    setError(null)
    try {
      await work()
    } catch (failure) {
      setError(errorMessage(failure as CommandError))
    } finally {
      setPending(false)
    }
  }

  const openSettings = () => {
    onClose()
    void navigate({ to: '/settings/$section', params: { section: 'forges' } })
  }

  const startFolder = async () => {
    const base = withTrailingSeparator(await rootFolder(), separator)
    await fetchListing(base)
    push({ kind: 'folder' }, base)
    return false
  }

  const sourceGroups = (): PaletteGroup[] => {
    const entries: (PaletteEntry & { search: string })[] = [
      {
        key: 'new',
        search: `${m.add_project_new()} ${m.add_project_new_description()} git init`,
        run: async () => {
          push({ kind: 'newProject' }, '')
          return false
        },
        content: (
          <SourceRow
            icon={<HugeiconsIcon icon={FolderGitIcon} strokeWidth={2} />}
            title={m.add_project_new()}
            description={m.add_project_new_description()}
          />
        )
      },
      {
        key: 'folder',
        search: `${m.add_project_local()} ${m.add_project_local_description()}`,
        run: startFolder,
        content: (
          <SourceRow
            icon={<HugeiconsIcon icon={FolderAddIcon} strokeWidth={2} />}
            title={m.add_project_local()}
            description={m.add_project_local_description()}
          />
        )
      },
      {
        key: 'url',
        search: `${m.add_project_git_url()} ${m.add_project_git_url_description()} clone`,
        run: async () => {
          push({ kind: 'repository', source: 'url' }, '')
          return false
        },
        content: (
          <SourceRow
            icon={<HugeiconsIcon icon={Link01Icon} strokeWidth={2} />}
            title={m.add_project_git_url()}
            description={m.add_project_git_url_description()}
          />
        )
      },
      ...FORGE_SOURCES.map((source) => {
        const signedOut = forgeLogin(accounts, source.forge) === null
        return {
          key: source.forge,
          search: `${source.title()} ${source.description()} clone`,
          disabled: signedOut,
          run: async () => {
            push({ kind: 'repository', source: source.forge }, '')
            return false
          },
          content: (
            <SourceRow
              icon={<ForgeIcon forge={source.forge} />}
              title={source.title()}
              description={source.description()}
              disabled={signedOut}
              trailing={
                signedOut ? (
                  <button
                    type='button'
                    title={m.add_project_setup_hint()}
                    onClick={(event) => {
                      event.stopPropagation()
                      openSettings()
                    }}
                    className='border-border text-muted-foreground hover:text-foreground hover:bg-muted shrink-0 cursor-pointer rounded-md border px-1.5 py-0.5 text-[0.6875rem] font-medium'
                  >
                    {m.add_project_setup_required()}
                  </button>
                ) : null
              }
            />
          )
        }
      })
    ]
    const needle = query.trim().toLowerCase()
    const found = entries.filter((entry) =>
      entry.search.toLowerCase().includes(needle)
    )
    return found.length > 0
      ? [{ label: m.add_project_sources(), entries: found }]
      : []
  }

  const sameName = (left: string, right: string) =>
    separator === '\\'
      ? left.toLowerCase() === right.toLowerCase()
      : left === right

  const cloneFolderName = step.kind === 'destination' ? name : ''
  const shownFolders = listing
    ? filterFolders(listing.folders, step.kind === 'folder' ? name : '')
    : []

  const folderGroups = (): PaletteGroup[] => {
    if (!listing) return []
    const into = (folder: string) =>
      step.kind === 'destination'
        ? joinPath(
            joinPath(directory, folder, separator),
            cloneFolderName,
            separator
          )
        : withTrailingSeparator(
            joinPath(directory, folder, separator),
            separator
          )
    const entries: PaletteEntry[] = []
    if (listing.parent !== null && (step.kind === 'destination' || !name)) {
      const parent = listing.parent
      entries.push({
        key: 'up',
        run: () =>
          browseTo(
            step.kind === 'destination'
              ? joinPath(parent, cloneFolderName, separator)
              : withTrailingSeparator(parent, separator)
          ),
        content: (
          <>
            <HugeiconsIcon
              icon={ArrowTurnBackwardIcon}
              strokeWidth={2}
              className={PALETTE_ICON}
            />
            <span className='font-mono text-xs'>..</span>
          </>
        )
      })
    }
    for (const folder of shownFolders.slice(0, FOLDER_LIMIT)) {
      entries.push({
        key: `folder:${folder}`,
        run: () => browseTo(into(folder)),
        content: (
          <>
            <HugeiconsIcon
              icon={Folder01Icon}
              strokeWidth={2}
              className={PALETTE_ICON}
            />
            <span className='truncate'>{folder}</span>
          </>
        )
      })
    }
    if (entries.length === 0) return []
    return [
      {
        label:
          step.kind === 'destination'
            ? m.add_project_clone_into()
            : m.add_project_folders(),
        entries
      }
    ]
  }

  const folderExists = endsWithSeparator(query, separator)
    ? listing !== null
    : listing?.folders.some((folder) => sameName(folder, name)) === true

  const submitFolder = () =>
    attempt(async () => {
      const path = resolveFolderQuery(query, separator)
      if (!isAbsolutePath(path, separator)) {
        setError(m.error_path_not_absolute())
        return
      }
      const created = folderExists
        ? path
        : await invoke<string>('create_folder', {
            path,
            initRepository: false
          })
      await finish(created)
    })

  const newProjectPath = root ? joinPath(root, query.trim(), separator) : null

  const submitNewProject = () =>
    attempt(async () => {
      if (!isValidFolderName(query)) {
        setError(m.error_project_name_invalid())
        return
      }
      if (!newProjectPath) return
      const created = await invoke<string>('create_folder', {
        path: newProjectPath,
        initRepository: true
      })
      await finish(created)
    })

  const submitRepository = (source: CloneSource) =>
    attempt(async () => {
      const input = query.trim()
      if (!input) return
      const base = await rootFolder()
      if (source === 'url') {
        const folder = repositoryFolderName(input)
        if (!folder || !/[/:]/.test(input)) {
          setError(m.error_git_url_invalid())
          return
        }
        await fetchListing(withTrailingSeparator(base, separator))
        push(
          {
            kind: 'destination',
            source,
            url: input,
            title: input,
            subtitle: null
          },
          joinPath(base, folder, separator)
        )
        return
      }
      const repository = await invoke<RemoteRepository>('find_repository', {
        forge: source,
        input
      })
      await fetchListing(withTrailingSeparator(base, separator))
      push(
        {
          kind: 'destination',
          source,
          url: repository.cloneUrl,
          title: repository.fullName,
          subtitle: repository.description ?? repository.webUrl
        },
        joinPath(base, repository.name, separator)
      )
    })

  const submitClone = (url: string) =>
    attempt(async () => {
      const destination = resolveFolderQuery(query, separator)
      if (!isAbsolutePath(destination, separator)) {
        setError(m.error_path_not_absolute())
        return
      }
      const cloned = await invoke<string>('clone_repository', {
        url,
        destination
      })
      await finish(cloned)
    })

  const pickWithSystemDialog = async () => {
    const defaultPath = listedDirectory ?? root ?? undefined
    const picked = await open({ directory: true, multiple: false, defaultPath })
    if (picked) await attempt(() => finish(picked))
  }

  const completeTab = (entry: PaletteEntry | undefined) => {
    if (entry) return void entry.run()
    if (step.kind !== 'folder' || !listing) return
    const completion = completeFolderName(shownFolders, name)
    if (!completion) return
    if (completion.complete) {
      void browseTo(
        withTrailingSeparator(
          joinPath(directory, completion.name, separator),
          separator
        )
      )
    } else {
      setQuery(`${directory}${completion.name}`)
    }
  }

  const highlighted = activeKey !== undefined
  const enterKeys = highlighted ? (isMac() ? '⌘ ↵' : 'Ctrl ↵') : '↵'

  let groups: PaletteGroup[] = []
  let placeholder: string = m.add_project_search()
  let empty: ReactNode = m.palette_no_results()
  let enterLabel: string = m.palette_select()
  let submit: (() => void) | undefined
  let submitLabel: string | null = null
  let summary: ReactNode = null

  switch (step.kind) {
    case 'sources':
      groups = sourceGroups()
      break
    case 'folder':
      groups = folderGroups()
      placeholder = m.add_project_path_placeholder()
      submit = () => void submitFolder()
      submitLabel = pending
        ? m.add_project_adding()
        : folderExists || loading
          ? m.add_project_add()
          : m.add_project_create_and_add()
      enterLabel = highlighted ? m.palette_select() : submitLabel
      empty = !query.trim()
        ? null
        : !listedDirectory
          ? m.error_path_not_absolute()
          : loading
            ? m.add_project_loading_folders()
            : folderExists
              ? m.add_project_no_folders()
              : m.add_project_will_create()
      break
    case 'newProject':
      placeholder = m.add_project_name_placeholder()
      submit = () => void submitNewProject()
      submitLabel = pending ? m.add_project_creating() : m.add_project_create()
      enterLabel = m.add_project_create()
      empty = null
      summary = (
        <Summary
          icon={<HugeiconsIcon icon={FolderGitIcon} strokeWidth={2} />}
          title={
            newProjectPath && query.trim()
              ? newProjectPath
              : m.add_project_new_location({ path: root ?? '…' })
          }
          subtitle={m.add_project_new_git()}
        />
      )
      break
    case 'repository': {
      const forge = FORGE_SOURCES.find((source) => source.forge === step.source)
      placeholder = forge
        ? forge.placeholder()
        : m.add_project_url_placeholder()
      submit = () => void submitRepository(step.source)
      submitLabel = pending
        ? m.add_project_looking_up()
        : forge
          ? m.add_project_look_up()
          : m.add_project_continue()
      enterLabel = forge ? m.add_project_look_up() : m.add_project_continue()
      empty = forge ? m.add_project_repository_hint() : m.add_project_url_hint()
      break
    }
    case 'destination':
      groups = folderGroups()
      placeholder = m.add_project_path_placeholder()
      submit = () => void submitClone(step.url)
      submitLabel = pending ? m.add_project_cloning() : m.add_project_clone()
      enterLabel = highlighted ? m.palette_select() : m.add_project_clone()
      empty = loading
        ? m.add_project_loading_folders()
        : m.add_project_clone_hint()
      summary = (
        <Summary
          icon={
            step.source === 'url' ? (
              <HugeiconsIcon icon={Link01Icon} strokeWidth={2} />
            ) : (
              <ForgeIcon forge={step.source} />
            )
          }
          title={step.title}
          subtitle={step.subtitle}
        />
      )
      break
  }

  return (
    <PaletteFrame
      key={stack.length}
      query={query}
      onQueryChange={setQuery}
      placeholder={placeholder}
      groups={groups}
      empty={empty}
      onClose={onClose}
      onBack={back}
      onSubmit={submit}
      onTab={browsing ? completeTab : undefined}
      autoHighlight={step.kind === 'sources'}
      enterLabel={enterLabel}
      onActiveChange={setActiveKey}
      accessory={
        submitLabel ? (
          <Button
            size='xs'
            variant='secondary'
            disabled={pending || !query.trim()}
            onClick={submit}
          >
            {submitLabel}
            {pending ? null : <Kbd>{enterKeys}</Kbd>}
          </Button>
        ) : null
      }
      status={
        error ? (
          <p
            role='alert'
            className='border-border text-destructive border-b px-4 py-2 text-xs text-pretty'
          >
            {error}
          </p>
        ) : null
      }
      footer={
        step.kind === 'folder' ? (
          <button
            type='button'
            disabled={pending}
            onClick={() => void pickWithSystemDialog()}
            className='hover:text-foreground cursor-pointer disabled:cursor-default disabled:opacity-50'
          >
            {fileManagerLabel(separator)}
          </button>
        ) : null
      }
    >
      {summary}
    </PaletteFrame>
  )
}

function SourceRow({
  icon,
  title,
  description,
  disabled,
  trailing
}: {
  icon: ReactNode
  title: string
  description: string
  disabled?: boolean
  trailing?: ReactNode
}) {
  return (
    <>
      <span
        className={cn(
          'text-muted-foreground flex size-4 shrink-0 items-center [&_svg]:size-4',
          disabled && 'opacity-60'
        )}
      >
        {icon}
      </span>
      <span className='flex min-w-0 flex-1 flex-col'>
        <span
          className={cn(
            'truncate',
            disabled ? 'text-muted-foreground' : 'font-medium'
          )}
        >
          {title}
        </span>
        <span className='text-muted-foreground truncate text-xs'>
          {description}
        </span>
      </span>
      {trailing}
    </>
  )
}

function Summary({
  icon,
  title,
  subtitle
}: {
  icon: ReactNode
  title: string
  subtitle: string | null
}) {
  return (
    <div className='border-border flex items-center gap-3 border-b px-4 py-3'>
      <span className='bg-muted text-muted-foreground flex size-8 shrink-0 items-center justify-center rounded-lg [&_svg]:size-4'>
        {icon}
      </span>
      <span className='flex min-w-0 flex-col'>
        <span className='truncate text-sm font-medium'>{title}</span>
        {subtitle ? (
          <span className='text-muted-foreground truncate text-xs'>
            {subtitle}
          </span>
        ) : null}
      </span>
    </div>
  )
}
