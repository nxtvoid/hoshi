/** Languages offered for a code block, by the id written after its opening fence. */
export const CODE_LANGUAGES = [
  { id: 'typescript', label: 'TypeScript', aliases: ['ts'] },
  { id: 'tsx', label: 'TSX', aliases: [] },
  { id: 'javascript', label: 'JavaScript', aliases: ['js'] },
  { id: 'jsx', label: 'JSX', aliases: [] },
  { id: 'json', label: 'JSON', aliases: [] },
  { id: 'rust', label: 'Rust', aliases: ['rs'] },
  { id: 'python', label: 'Python', aliases: ['py'] },
  { id: 'go', label: 'Go', aliases: [] },
  { id: 'java', label: 'Java', aliases: [] },
  { id: 'kotlin', label: 'Kotlin', aliases: ['kt'] },
  { id: 'swift', label: 'Swift', aliases: [] },
  { id: 'c', label: 'C', aliases: [] },
  { id: 'cpp', label: 'C++', aliases: ['c++'] },
  { id: 'csharp', label: 'C#', aliases: ['cs', 'c#'] },
  { id: 'ruby', label: 'Ruby', aliases: ['rb'] },
  { id: 'php', label: 'PHP', aliases: [] },
  { id: 'html', label: 'HTML', aliases: [] },
  { id: 'css', label: 'CSS', aliases: [] },
  { id: 'scss', label: 'SCSS', aliases: [] },
  { id: 'sql', label: 'SQL', aliases: [] },
  { id: 'bash', label: 'Bash', aliases: ['sh', 'shell', 'zsh'] },
  { id: 'powershell', label: 'PowerShell', aliases: ['ps1', 'pwsh'] },
  { id: 'yaml', label: 'YAML', aliases: ['yml'] },
  { id: 'toml', label: 'TOML', aliases: [] },
  { id: 'markdown', label: 'Markdown', aliases: ['md'] },
  { id: 'diff', label: 'Diff', aliases: [] },
  { id: 'dockerfile', label: 'Dockerfile', aliases: ['docker'] },
  { id: 'xml', label: 'XML', aliases: [] },
  { id: 'lua', label: 'Lua', aliases: [] }
] as const

type CodeLanguage = (typeof CODE_LANGUAGES)[number]

/** The offered language an id or alias names, ignoring case. */
export function codeLanguage(id: string): CodeLanguage | undefined {
  const needle = id.trim().toLowerCase()
  return CODE_LANGUAGES.find(
    (language) =>
      language.id === needle ||
      language.aliases.some((alias) => alias === needle)
  )
}

/** How a block's language is shown: its name, or the id as written. */
export function codeLanguageLabel(id: string) {
  return codeLanguage(id)?.label ?? id
}

/** The languages whose name or id contains `query`. */
export function searchCodeLanguages(query: string): readonly CodeLanguage[] {
  const needle = query.trim().toLowerCase()
  if (!needle) return CODE_LANGUAGES
  return CODE_LANGUAGES.filter(
    (language) =>
      language.label.toLowerCase().includes(needle) ||
      language.id.includes(needle) ||
      language.aliases.some((alias) => alias.includes(needle))
  )
}

/** The language of an opening fence typed on its own line, like ```tsx; `null` otherwise. */
export function fenceLanguage(line: string): string | null {
  const fence = /^```([\w+#.-]*)$/.exec(line.trim())
  return fence ? (fence[1] ?? '') : null
}
