import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { HugeiconsIcon } from '@hugeicons/react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { Input } from '@hoshi/ui/components/input'
import { Button } from '@hoshi/ui/components/button'
import { useThemeStore } from '@/state/theme-store'
import { themeErrorMessage } from './theme-error-messages'
import type { ExtensionTheme } from '@/bindings/ExtensionTheme'
import type { ThemeExtension } from '@/bindings/ThemeExtension'
import type { ThemeExtensionSort } from '@/bindings/ThemeExtensionSort'
import {
  type ChangeEvent,
  type DragEvent,
  type UIEvent,
  useEffect,
  useId,
  useRef,
  useState
} from 'react'
import { Dialog, DialogContent, DialogTitle } from '@hoshi/ui/components/dialog'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle
} from '@hoshi/ui/components/alert-dialog'
import {
  Add01Icon,
  LinkSquare02Icon,
  Loading03Icon,
  PackageAddIcon,
  PaintBoardIcon,
  RefreshIcon,
  Search01Icon
} from '@hugeicons/core-free-icons'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue
} from '@hoshi/ui/components/select'
import {
  extensionThemes,
  humanizeThemeName,
  pairVsCodeThemes,
  parseThemeImport,
  resolveThemeLabelCollisions
} from '@/lib/vscode-theme'
import {
  installTheme,
  parseThemeFile,
  replaceCollection,
  selectionWithout,
  THEME_FILE_VERSION,
  THEME_NAME_MAX,
  type ThemeDefinition,
  ThemeError,
  type ThemeSelection,
  updateTheme
} from '@/state/themes'

const MAX_THEME_FILE_BYTES = 256 * 1024
const MAX_HIGHLIGHTED_LENGTH = 20_000
const SEARCH_DEBOUNCE_MS = 350
const SUGGESTED_SEARCHES = ['Dracula', 'Catppuccin', 'Nord', 'Tokyo Night']
const DOWNLOADS = new Intl.NumberFormat(undefined, {
  notation: 'compact',
  maximumFractionDigits: 1
})

const SORT_OPTIONS: readonly {
  value: ThemeExtensionSort
  label: () => string
}[] = [
  { value: 'downloadCount', label: m.sort_most_downloaded },
  { value: 'rating', label: m.sort_best_rated },
  { value: 'timestamp', label: m.sort_newest },
  { value: 'relevance', label: m.sort_most_relevant }
]

type ImportFailure = { code: string }

function importErrorMessage(cause: unknown): string {
  if (cause instanceof ThemeError) return themeErrorMessage(cause)
  const code =
    typeof cause === 'string' ? cause : (cause as ImportFailure)?.code
  switch (code) {
    case 'search_unavailable':
      return m.open_vsx_search_unavailable()
    case 'search_unreadable':
    case 'details_unreadable':
      return m.open_vsx_unreadable()
    case 'details_unavailable':
      return m.open_vsx_details_unavailable()
    case 'no_manifest':
      return m.open_vsx_no_manifest()
    case 'no_themes':
      return m.open_vsx_no_themes()
    case 'too_many_themes':
      return m.open_vsx_too_many_themes()
    case 'download_failed':
      return m.open_vsx_download_failed()
    case 'no_checksum':
    case 'invalid_checksum':
      return m.open_vsx_no_checksum()
    case 'integrity_failed':
      return m.open_vsx_integrity_failed()
    case 'unsafe_package':
      return m.open_vsx_unsafe_package()
    case 'package_mismatch':
    case 'license_mismatch':
      return m.open_vsx_package_mismatch()
    default:
      return m.open_vsx_theme_unreadable()
  }
}

function sizeLabel(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`
  return `${bytes} B`
}

function oversized(bytes: number): string | null {
  return bytes > MAX_THEME_FILE_BYTES
    ? m.theme_file_too_large({
        size: sizeLabel(bytes),
        limit: sizeLabel(MAX_THEME_FILE_BYTES)
      })
    : null
}

function escapeHtml(value: string): string {
  return value.replace(
    /[&<>"']/g,
    (c) =>
      ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[
        c
      ] ?? c
  )
}

function highlightJson(value: string): string {
  const pattern =
    /"(?:\\.|[^"\\])*"|-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?|true|false|null/g
  let html = ''
  let cursor = 0
  for (const match of value.matchAll(pattern)) {
    const token = match[0]
    const index = match.index ?? 0
    html += escapeHtml(value.slice(cursor, index))
    let tone = 'text-file-orange'
    if (token.startsWith('"')) {
      tone = /^\s*:/.test(value.slice(index + token.length))
        ? 'text-file-blue'
        : 'text-file-green'
    } else if (token === 'true' || token === 'false' || token === 'null') {
      tone = 'text-file-purple'
    }
    html += `<span class="${tone}">${escapeHtml(token)}</span>`
    cursor = index + token.length
  }
  return html + escapeHtml(value.slice(cursor))
}

function ThemeJsonEditor({
  id,
  value,
  onChange
}: {
  id: string
  value: string
  onChange: (value: string) => void
}) {
  const highlightRef = useRef<HTMLPreElement>(null)
  const plain = value.length > MAX_HIGHLIGHTED_LENGTH
  const syncScroll = (event: UIEvent<HTMLTextAreaElement>) => {
    const highlight = highlightRef.current
    if (!highlight) return
    highlight.scrollTop = event.currentTarget.scrollTop
    highlight.scrollLeft = event.currentTarget.scrollLeft
  }
  return (
    <div className='border-input bg-background focus-within:border-ring focus-within:ring-ring/50 relative overflow-hidden rounded-lg border focus-within:ring-3'>
      {plain ? null : (
        <pre
          ref={highlightRef}
          aria-hidden
          className='text-foreground pointer-events-none absolute inset-0 m-0 overflow-hidden p-3 font-mono text-xs leading-5 break-words whitespace-pre-wrap'
        >
          <code dangerouslySetInnerHTML={{ __html: highlightJson(value) }} />
        </pre>
      )}
      <textarea
        id={id}
        spellCheck={false}
        value={value}
        placeholder={
          '{\n  "version": 1,\n  "name": "Aurora",\n  "appearance": "light",\n  "colors": { ... }\n}'
        }
        onChange={(event) => onChange(event.currentTarget.value)}
        onScroll={syncScroll}
        className={cn(
          'caret-foreground placeholder:text-muted-foreground selection:bg-primary selection:text-primary-foreground relative z-10 block min-h-44 w-full resize-y overflow-auto bg-transparent p-3 font-mono text-xs leading-5 outline-none',
          plain ? 'text-foreground' : 'text-transparent'
        )}
      />
    </div>
  )
}

type ImportFile = { name: string; size: number; text: () => Promise<string> }

/** A theme that took one half when it has one appearance, or the whole choice. */
export type ImportedSelection = (
  theme: ThemeDefinition,
  selection: ThemeSelection
) => ThemeSelection

export function ThemeImportDialog({
  open,
  onOpenChange,
  chooseImported
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  chooseImported: ImportedSelection
}) {
  const [session, setSession] = useState(0)
  const [seenOpen, setSeenOpen] = useState(open)
  if (seenOpen !== open) {
    setSeenOpen(open)
    if (open) setSession(session + 1)
  }
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className='flex max-h-[calc(100dvh-4rem)] flex-col gap-4 overflow-hidden sm:max-w-3xl'>
        <DialogTitle className='text-base font-semibold'>
          {m.add_a_theme()}
        </DialogTitle>
        <ThemeImportBody
          key={session}
          onClose={() => onOpenChange(false)}
          chooseImported={chooseImported}
        />
      </DialogContent>
    </Dialog>
  )
}

function ThemeImportBody({
  onClose,
  chooseImported
}: {
  onClose: () => void
  chooseImported: ImportedSelection
}) {
  const id = useId()
  const fileInput = useRef<HTMLInputElement>(null)
  const [json, setJson] = useState('')
  const [fileName, setFileName] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [reading, setReading] = useState(false)
  const [dropTarget, setDropTarget] = useState(false)
  const [conflicts, setConflicts] = useState<ThemeDefinition[] | null>(null)
  const request = useRef(0)
  const alive = useRef(true)
  useEffect(
    () => () => {
      alive.current = false
    },
    []
  )
  const current = (requestId: number) =>
    alive.current && requestId === request.current
  const store = () => useThemeStore.getState()
  const fallbackName = m.vscode_theme()

  const readFile = async (file: ImportFile) => {
    const tooLarge = oversized(file.size)
    if (tooLarge) {
      setError(tooLarge)
      return
    }
    const requestId = ++request.current
    setReading(true)
    try {
      const text = await file.text()
      if (!current(requestId)) return
      setJson(text)
      setFileName(file.name)
      setError(null)
    } catch {
      if (current(requestId)) setError(m.theme_file_unreadable())
    } finally {
      if (current(requestId)) setReading(false)
    }
  }

  const installBatch = async (files: readonly ImportFile[]) => {
    const requestId = ++request.current
    setReading(true)
    const failures: string[] = []
    const parsed: { theme: ThemeDefinition; sourceName: string }[] = []
    for (const file of files) {
      if (oversized(file.size)) {
        failures.push(
          m.theme_file_failure({ name: file.name, reason: m.too_large() })
        )
        continue
      }
      try {
        parsed.push({
          sourceName: file.name,
          theme: parseThemeImport(await file.text(), fallbackName)
        })
      } catch (cause) {
        failures.push(
          m.theme_file_failure({
            name: file.name,
            reason: importErrorMessage(cause)
          })
        )
      }
    }
    if (!current(requestId)) return
    setReading(false)
    let themes = store().customThemes
    let installed = 0
    const conflicting: ThemeDefinition[] = []
    for (const theme of pairVsCodeThemes(resolveThemeLabelCollisions(parsed))) {
      if (themes.some((existing) => existing.id === theme.id)) {
        conflicting.push(theme)
        continue
      }
      try {
        themes = installTheme(themes, theme)
        installed += 1
      } catch (cause) {
        failures.push(
          m.theme_file_failure({
            name: theme.label,
            reason: importErrorMessage(cause)
          })
        )
      }
    }
    if (installed > 0) store().setCustomThemes(themes)
    setError(failures.length > 0 ? failures.join('; ') : null)
    if (conflicting.length > 0) setConflicts(conflicting)
    else if (installed > 0 && failures.length === 0) onClose()
  }

  const readFiles = (files: readonly ImportFile[]) => {
    if (files.length === 0) return
    if (files.length === 1) void readFile(files[0]!)
    else void installBatch(files)
  }

  const handleFileChange = (event: ChangeEvent<HTMLInputElement>) => {
    const files = [...(event.currentTarget.files ?? [])]
    event.currentTarget.value = ''
    readFiles(files)
  }

  const handleDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    setDropTarget(false)
    readFiles([...event.dataTransfer.files])
  }

  const versionedCopy = (
    theme: ThemeDefinition,
    themes: readonly ThemeDefinition[],
    preferredName: string | null
  ): ThemeDefinition => {
    const copy = (name: string) =>
      parseThemeFile({
        version: THEME_FILE_VERSION,
        name,
        appearance: theme.appearance,
        colors: theme.colors,
        ...(theme.variants ? { variants: theme.variants } : {}),
        ...(theme.managed ? { managed: true } : {})
      })
    const free = (candidate: ThemeDefinition) =>
      !themes.some((existing) => existing.id === candidate.id)
    if (
      preferredName &&
      preferredName.toLowerCase() !== theme.label.toLowerCase()
    ) {
      const candidate = copy(preferredName.slice(0, THEME_NAME_MAX))
      if (free(candidate)) return candidate
    }
    for (let count = 1; count < 100; count += 1) {
      const tail = ` (${count})`
      const candidate = copy(
        `${theme.label.slice(0, THEME_NAME_MAX - tail.length)}${tail}`
      )
      if (free(candidate)) return candidate
    }
    throw new ThemeError('already_installed', theme.label)
  }

  const resolveConflicts = (mode: 'update' | 'copy') => {
    if (!conflicts) return
    const failures: string[] = []
    const preferredName =
      conflicts.length === 1 && fileName
        ? humanizeThemeName(fileName.replace(/\.[^.]+$/, ''))
        : null
    let themes = store().customThemes
    for (const theme of conflicts) {
      try {
        if (mode === 'update') {
          const existing = themes.find((candidate) => candidate.id === theme.id)
          themes = updateTheme(
            themes,
            existing?.collection
              ? { ...theme, collection: existing.collection }
              : theme
          )
        } else {
          themes = installTheme(
            themes,
            versionedCopy(theme, themes, preferredName)
          )
        }
      } catch (cause) {
        failures.push(
          m.theme_file_failure({
            name: theme.label,
            reason: importErrorMessage(cause)
          })
        )
      }
    }
    store().setCustomThemes(themes)
    setConflicts(null)
    if (failures.length > 0) {
      setError([error, ...failures].filter(Boolean).join('; '))
    } else if (!error) onClose()
  }

  const submit = () => {
    const tooLarge = oversized(json.length)
    if (tooLarge) {
      setError(tooLarge)
      return
    }
    try {
      const theme = parseThemeImport(json, fallbackName)
      const { customThemes, theme: base, halves, setCustomThemes } = store()
      if (customThemes.some((existing) => existing.id === theme.id)) {
        setError(null)
        setConflicts([theme])
        return
      }
      setCustomThemes(
        installTheme(customThemes, theme),
        chooseImported(theme, { theme: base, halves })
      )
      onClose()
    } catch (cause) {
      setError(importErrorMessage(cause))
    }
  }

  return (
    <div className='-mx-4 flex min-h-0 flex-col gap-5 overflow-y-auto px-4 pb-1'>
      <ThemeSearchSection onInstalled={onClose} />
      <div aria-hidden className='flex items-center gap-3'>
        <div className='bg-border h-px flex-1' />
        <span className='text-muted-foreground text-xs tracking-wider uppercase'>
          {m.or()}
        </span>
        <div className='bg-border h-px flex-1' />
      </div>
      {conflicts ? (
        <div className='flex flex-col gap-3'>
          <div className='border-border/70 bg-muted/30 rounded-xl border p-3'>
            <p className='text-sm font-medium'>{m.theme_already_installed()}</p>
            <p className='text-muted-foreground mt-1 text-xs'>
              {conflicts.map((theme) => theme.label).join(', ')}
            </p>
          </div>
          <div className='flex flex-wrap items-center gap-2'>
            <Button size='sm' onClick={() => resolveConflicts('update')}>
              {m.update_existing()}
            </Button>
            <Button
              size='sm'
              variant='outline'
              onClick={() => resolveConflicts('copy')}
            >
              {m.keep_both()}
            </Button>
            <Button
              size='sm'
              variant='ghost'
              onClick={() => setConflicts(null)}
            >
              {m.back()}
            </Button>
            <Button size='sm' variant='ghost' onClick={onClose}>
              {m.cancel()}
            </Button>
          </div>
        </div>
      ) : (
        <div className='flex flex-col gap-4'>
          {/* biome-ignore lint/a11y/noStaticElementInteractions: a drop zone; the button inside is the keyboard path */}
          <div
            onDragEnter={(event) => {
              event.preventDefault()
              setDropTarget(true)
            }}
            onDragOver={(event) => event.preventDefault()}
            onDragLeave={(event) => {
              if (!event.currentTarget.contains(event.relatedTarget as Node)) {
                setDropTarget(false)
              }
            }}
            onDrop={handleDrop}
            className={cn(
              'flex flex-wrap items-center justify-between gap-3 rounded-xl border border-dashed px-3 py-3 transition-colors',
              dropTarget
                ? 'border-ring bg-accent/40'
                : 'border-border bg-muted/20'
            )}
          >
            <div className='min-w-0'>
              <p className='text-sm font-medium'>{m.theme_file()}</p>
              <p className='text-muted-foreground truncate text-xs'>
                {fileName ?? m.theme_file_drop_hint()}
              </p>
            </div>
            <Button
              size='sm'
              variant='outline'
              disabled={reading}
              onClick={() => fileInput.current?.click()}
            >
              {reading ? (
                <HugeiconsIcon
                  icon={Loading03Icon}
                  strokeWidth={2}
                  className='animate-spin'
                />
              ) : null}
              {m.choose_files()}
            </Button>
            <input
              ref={fileInput}
              type='file'
              accept='.json,.jsonc,application/json'
              multiple
              hidden
              onChange={handleFileChange}
            />
          </div>
          <div className='flex flex-col gap-2'>
            <label htmlFor={`${id}-json`} className='text-sm font-medium'>
              {m.theme_json()}
            </label>
            <ThemeJsonEditor
              id={`${id}-json`}
              value={json}
              onChange={setJson}
            />
          </div>
          <div className='flex flex-col-reverse gap-2 sm:flex-row sm:justify-end'>
            <Button variant='ghost' onClick={onClose}>
              {m.cancel()}
            </Button>
            <Button disabled={!json.trim() || reading} onClick={submit}>
              <HugeiconsIcon icon={Add01Icon} strokeWidth={2} />
              {m.add_theme()}
            </Button>
          </div>
        </div>
      )}
      {error ? (
        <p
          aria-live='polite'
          className='border-destructive/30 bg-destructive/10 text-destructive rounded-lg border px-3 py-2 text-sm'
        >
          {error}
        </p>
      ) : null}
    </div>
  )
}

function ExtensionIcon({ extension }: { extension: ThemeExtension }) {
  const [failed, setFailed] = useState(false)
  return (
    <div className='bg-muted text-muted-foreground flex size-10 shrink-0 items-center justify-center overflow-hidden rounded-lg'>
      {extension.iconUrl && !failed ? (
        <img
          alt=''
          className='size-full object-cover'
          loading='lazy'
          referrerPolicy='no-referrer'
          src={extension.iconUrl}
          onError={() => setFailed(true)}
        />
      ) : (
        <HugeiconsIcon
          icon={PaintBoardIcon}
          strokeWidth={2}
          className='size-4'
        />
      )}
    </div>
  )
}

function ThemeSearchSection({ onInstalled }: { onInstalled: () => void }) {
  const [query, setQuery] = useState('')
  const [sortBy, setSortBy] = useState<ThemeExtensionSort>('downloadCount')
  const [results, setResults] = useState<ThemeExtension[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [searching, setSearching] = useState(false)
  const [installing, setInstalling] = useState<string | null>(null)
  const [pendingUpdate, setPendingUpdate] = useState<ThemeExtension | null>(
    null
  )
  const customThemes = useThemeStore((state) => state.customThemes)
  const searchRequest = useRef(0)
  const debounce = useRef<ReturnType<typeof setTimeout> | null>(null)
  const alive = useRef(true)
  useEffect(
    () => () => {
      alive.current = false
      if (debounce.current) clearTimeout(debounce.current)
    },
    []
  )

  const runSearch = (text: string, sort: ThemeExtensionSort) => {
    const requestId = ++searchRequest.current
    const trimmed = text.trim()
    if (!trimmed) {
      setResults(null)
      setError(null)
      setSearching(false)
      return
    }
    setSearching(true)
    setError(null)
    invoke<ThemeExtension[]>('search_theme_extensions', {
      query: trimmed,
      sortBy: sort
    })
      .then((found) => {
        if (!alive.current || requestId !== searchRequest.current) return
        setResults(found)
      })
      .catch((cause: unknown) => {
        if (!alive.current || requestId !== searchRequest.current) return
        setResults(null)
        setError(importErrorMessage(cause))
      })
      .finally(() => {
        if (alive.current && requestId === searchRequest.current) {
          setSearching(false)
        }
      })
  }

  const changeQuery = (next: string) => {
    setQuery(next)
    if (debounce.current) clearTimeout(debounce.current)
    if (!next.trim()) {
      runSearch('', sortBy)
      return
    }
    debounce.current = setTimeout(
      () => runSearch(next, sortBy),
      SEARCH_DEBOUNCE_MS
    )
  }

  const install = async (extension: ThemeExtension, allowUpdate: boolean) => {
    setError(null)
    const installedCollection = useThemeStore
      .getState()
      .customThemes.filter(
        (theme) => theme.collection?.id === extension.collectionId
      )
    if (installedCollection.length > 0 && !allowUpdate) {
      setPendingUpdate(extension)
      return
    }
    searchRequest.current += 1
    setSearching(false)
    setInstalling(extension.id)
    try {
      const files = await invoke<ExtensionTheme[]>('import_theme_extension', {
        extension
      })
      if (!alive.current) return
      const themes = extensionThemes(extension, files)
      const {
        customThemes: current,
        theme: base,
        halves,
        setCustomThemes
      } = useThemeStore.getState()
      const stillInstalled = current.filter(
        (candidate) => candidate.collection?.id === extension.collectionId
      )
      if (
        JSON.stringify(stillInstalled) !== JSON.stringify(installedCollection)
      ) {
        throw new ThemeError('collection_changed')
      }
      const removed = stillInstalled
        .filter((old) => !themes.some((added) => added.id === old.id))
        .map((old) => old.id)
      setCustomThemes(
        replaceCollection(current, extension.collectionId, themes),
        removed.length > 0
          ? selectionWithout({ theme: base, halves }, removed)
          : undefined
      )
      onInstalled()
    } catch (cause) {
      if (alive.current) setError(importErrorMessage(cause))
    } finally {
      if (alive.current) setInstalling(null)
    }
  }

  return (
    <section
      className='flex flex-col gap-3'
      aria-labelledby='theme-search-heading'
    >
      <div>
        <h3 className='text-sm font-medium' id='theme-search-heading'>
          {m.search_community_themes()}
        </h3>
        <p className='text-muted-foreground mt-0.5 text-xs'>
          {m.search_community_themes_description()}
        </p>
      </div>
      <div className='relative'>
        <HugeiconsIcon
          icon={searching ? Loading03Icon : Search01Icon}
          strokeWidth={2}
          className={cn(
            'text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2',
            searching && 'animate-spin'
          )}
        />
        <Input
          type='search'
          autoFocus
          aria-label={m.search_open_vsx_themes()}
          placeholder={m.search_themes_placeholder()}
          className='h-9 ps-8'
          value={query}
          onChange={(event) => changeQuery(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing) return
            if (event.key === 'Enter' && installing === null) {
              if (debounce.current) clearTimeout(debounce.current)
              runSearch(query, sortBy)
            }
          }}
        />
      </div>
      {!searching || results !== null ? (
        <div className='flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between'>
          <div className='flex min-w-0 flex-wrap items-center gap-1.5'>
            <p className='text-muted-foreground text-xs'>{m.popular()}</p>
            {SUGGESTED_SEARCHES.map((suggestion) => (
              <Button
                key={suggestion}
                size='xs'
                variant='ghost'
                disabled={installing !== null}
                onClick={() => {
                  if (query.trim() === suggestion) runSearch(suggestion, sortBy)
                  else {
                    setQuery(suggestion)
                    if (debounce.current) clearTimeout(debounce.current)
                    runSearch(suggestion, sortBy)
                  }
                }}
              >
                {suggestion}
              </Button>
            ))}
          </div>
          {results && results.length > 0 ? (
            <div className='flex shrink-0 items-center justify-end gap-2'>
              <p className='text-muted-foreground text-xs'>{m.sort()}</p>
              <Select
                items={SORT_OPTIONS.map((option) => ({
                  value: option.value,
                  label: option.label()
                }))}
                value={sortBy}
                disabled={installing !== null}
                onValueChange={(next: ThemeExtensionSort | null) => {
                  if (!next) return
                  setSortBy(next)
                  runSearch(query, next)
                }}
              >
                <SelectTrigger aria-label={m.sort_themes()} className='w-40'>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {SORT_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {option.label()}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          ) : null}
        </div>
      ) : null}
      <div className='sr-only' role='status'>
        {searching
          ? m.searching_themes()
          : results
            ? m.themes_found({ count: results.length })
            : ''}
      </div>
      {error ? (
        <p
          aria-live='polite'
          className='border-destructive/30 bg-destructive/10 text-destructive rounded-lg border px-3 py-2 text-sm'
        >
          {error}
        </p>
      ) : null}
      {searching && results === null ? (
        <div className='text-muted-foreground flex min-h-20 items-center justify-center gap-2 text-sm'>
          <HugeiconsIcon
            icon={Loading03Icon}
            strokeWidth={2}
            className='size-4 animate-spin'
          />
          {m.searching_themes()}
        </div>
      ) : null}
      {results ? (
        results.length === 0 ? (
          <div className='flex min-h-40 flex-col items-center justify-center rounded-2xl border border-dashed text-center'>
            <p className='text-sm font-medium'>{m.no_open_source_themes()}</p>
            <p className='text-muted-foreground mt-1 text-xs'>
              {m.try_broader_search()}
            </p>
          </div>
        ) : (
          <div className='grid gap-2 sm:grid-cols-2'>
            {results.map((extension) => {
              const busy = installing === extension.id
              const installed = customThemes.some(
                (theme) => theme.collection?.id === extension.collectionId
              )
              return (
                <article
                  key={extension.id}
                  className='border-border/70 bg-card/60 hover:bg-muted/30 flex min-w-0 flex-col gap-3 rounded-xl border p-3 transition-colors'
                >
                  <div className='flex min-w-0 gap-3'>
                    <ExtensionIcon
                      key={extension.iconUrl}
                      extension={extension}
                    />
                    <div className='min-w-0 flex-1'>
                      <h4 className='truncate text-sm font-medium'>
                        {extension.name}
                      </h4>
                      <p className='text-muted-foreground truncate text-xs'>
                        {m.extension_byline({
                          publisher: extension.publisher,
                          downloads: DOWNLOADS.format(extension.downloadCount)
                        })}
                      </p>
                    </div>
                  </div>
                  <p className='text-muted-foreground line-clamp-2 min-h-8 text-xs leading-4'>
                    {extension.description || m.community_theme_description()}
                  </p>
                  <div className='mt-auto flex items-center justify-between gap-2'>
                    <div className='flex min-w-0 items-center gap-2'>
                      {extension.sourceUrl ? (
                        <Button
                          variant='ghost'
                          size='icon-xs'
                          aria-label={m.view_source_for({
                            name: extension.name
                          })}
                          onClick={() => {
                            if (extension.sourceUrl)
                              void openUrl(extension.sourceUrl)
                          }}
                        >
                          <HugeiconsIcon
                            icon={LinkSquare02Icon}
                            strokeWidth={2}
                          />
                        </Button>
                      ) : null}
                    </div>
                    <Button
                      size='xs'
                      variant='outline'
                      disabled={installing !== null}
                      onClick={() => void install(extension, false)}
                    >
                      <HugeiconsIcon
                        icon={
                          busy
                            ? Loading03Icon
                            : installed
                              ? RefreshIcon
                              : PackageAddIcon
                        }
                        strokeWidth={2}
                        className={cn(busy && 'animate-spin')}
                      />
                      {busy
                        ? installed
                          ? m.updating()
                          : m.installing()
                        : installed
                          ? m.update()
                          : m.install()}
                    </Button>
                  </div>
                </article>
              )
            })}
          </div>
        )
      ) : null}
      <AlertDialog
        open={pendingUpdate !== null}
        onOpenChange={(next) => {
          if (!next) setPendingUpdate(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {m.update_theme_confirm({ name: pendingUpdate?.name ?? '' })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {m.update_theme_description()}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{m.cancel()}</AlertDialogCancel>
            <Button
              onClick={() => {
                const extension = pendingUpdate
                setPendingUpdate(null)
                if (extension) void install(extension, true)
              }}
            >
              {m.update_theme()}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  )
}
