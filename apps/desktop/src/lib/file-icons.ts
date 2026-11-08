import { createFileTreeIconResolver } from '@pierre/trees'

const resolver = createFileTreeIconResolver({ set: 'complete', colored: true })

/** The theme's color for each kind of file the icon set knows. */
const COLORS = {
  astro: 'text-file-magenta',
  babel: 'text-file-yellow',
  bash: 'text-file-green',
  biome: 'text-file-blue',
  bootstrap: 'text-file-purple',
  browserslist: 'text-file-yellow',
  bun: 'text-file-gray',
  c: 'text-file-blue',
  claude: 'text-file-orange',
  cpp: 'text-file-blue',
  css: 'text-file-purple',
  database: 'text-file-magenta',
  default: 'text-file-gray',
  docker: 'text-file-blue',
  eslint: 'text-file-purple',
  font: 'text-file-gray',
  git: 'text-file-orange',
  go: 'text-file-cyan',
  graphql: 'text-file-pink',
  html: 'text-file-orange',
  image: 'text-file-pink',
  javascript: 'text-file-yellow',
  json: 'text-file-orange',
  markdown: 'text-file-green',
  mcp: 'text-file-teal',
  nextjs: 'text-file-gray',
  npm: 'text-file-red',
  oxc: 'text-file-cyan',
  postcss: 'text-file-red',
  prettier: 'text-file-teal',
  python: 'text-file-blue',
  react: 'text-file-cyan',
  ruby: 'text-file-red',
  rust: 'text-file-orange',
  sass: 'text-file-pink',
  stylelint: 'text-file-gray',
  svelte: 'text-file-red',
  svg: 'text-file-orange',
  svgo: 'text-file-green',
  swift: 'text-file-orange',
  table: 'text-file-teal',
  tailwind: 'text-file-cyan',
  terraform: 'text-file-purple',
  text: 'text-file-gray',
  typescript: 'text-file-blue',
  vite: 'text-file-magenta',
  vscode: 'text-file-blue',
  vue: 'text-file-green',
  wasm: 'text-file-purple',
  webpack: 'text-file-blue',
  yml: 'text-file-red',
  zig: 'text-file-orange',
  zip: 'text-file-orange'
} satisfies Record<string, string>

/** The sprite symbol for a file and the class that colors it. */
export function fileIcon(path: string): { name: string; color: string } {
  const icon = resolver.resolveIcon('file-tree-icon-file', path)
  const token = (icon.token ?? 'default') as keyof typeof COLORS
  return { name: icon.name, color: COLORS[token] ?? COLORS.default }
}

const EXTENSIONS: Record<string, string> = {
  typescript: 'ts',
  javascript: 'js',
  python: 'py',
  rust: 'rs',
  ruby: 'rb',
  kotlin: 'kt',
  csharp: 'cs',
  bash: 'sh',
  powershell: 'ps1',
  markdown: 'md',
  '': 'txt'
}

/** A file name in a code block's language, to show that language's icon. */
export function languageFileName(language: string) {
  const id = language.trim().toLowerCase()
  if (id === 'dockerfile') return 'Dockerfile'
  return `file.${EXTENSIONS[id] ?? id}`
}
