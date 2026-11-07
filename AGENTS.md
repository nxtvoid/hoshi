# hoshi

## Layout

- `apps/desktop/src` - React + Tailwind v4 UI. Renders state; holds no business logic.
- `apps/desktop/src-tauri` - Thin Tauri shell: forwards commands to the engine and events to the UI.
- `apps/desktop/src/bindings` - TypeScript generated from `hoshi-core` by ts-rs. Never edit by hand.
- `apps/desktop/messages` - UI text per locale, compiled by Paraglide JS into `src/paraglide` (git-ignored).
- `crates/hoshi-core` - Domain model and orchestrator. Pure: commands in, events out. No I/O.
  Its types are the contract with the UI.
- `crates/hoshi-engine` - Runtime that wires core, store and providers: persists events, runs effects.
- `crates/hoshi-store` - SQLite persistence for events and projections.
- `crates/hoshi-providers` - One adapter per agent CLI behind a common trait. Claude Code first, Codex next.

## Commands

- `bun i` installs. `bun run dev` starts the app. `bun run build` bundles it.
- `bun run check` runs Biome, rustfmt and clippy. `bun run fix` applies formatting.
- `bun run typecheck` and `bun run test` (Rust tests).
- `bun run bindings` regenerates the TypeScript bindings. Run it after changing a `#[ts(export)]` type.
- `bun run --cwd apps/desktop check:i18n` checks the translations.

### Not decided yet — ask before establishing a pattern

UI component library, client state management, routing, where the SQLite database lives, and the error type for library crates.

## CodeGraph

The repo is indexed by [CodeGraph](https://github.com/colbymchenry/codegraph). Only `.codegraph/.gitignore` is committed; the index (`.codegraph/codegraph.db`) is local to each machine. Set it up once per machine with `npm i -g @colbymchenry/codegraph` and `codegraph init -y` at the repo root; while an agent runs its MCP server, a file watcher keeps the index in sync. To understand or locate code, query it before grepping or reading files: the `codegraph_explore` MCP tool, or `codegraph explore "<symbols or question>"` in a shell.

## Architecture rules

- Complexity belongs in the provider adapters. The orchestrator stays pure and the UI stays dumb.
- Adding a provider means implementing the adapter trait; nothing else should change.
- A thread runs in its own working directory. A thread on a worktree spawns its agent
  with that worktree as `cwd`, never the project root.
- Performance matters: long sessions and many parallel sessions must stay smooth.
  Virtualize long lists and do not ship whole transcripts across the IPC bridge.
- Types that cross to the UI derive `Serialize` and `TS` with `#[ts(export)]` and use camelCase:
  `rename_all = "camelCase"` on structs; on enums also `rename_all_fields = "camelCase"`.

## Code conventions

- **Identifiers in English:** types, functions, variables, enum values, file names.
- **No narrating comments:** do not write comments that explain why the code does something or restate a rule. Keep a short comment only for genuinely non-obvious intent. Applies to Rust and TypeScript.
- **TypeScript** follows `biome.json` (single quotes, no semicolons). **Rust** follows rustfmt and clippy; `bun run check` fails on any clippy warning.
- **Import order (TypeScript):** side-effect imports and namespace imports (`import * as React`) first, then named imports from shortest to longest line, then multiline imports, then default imports:

```ts
import './globals.css'
import { cn } from '@/lib/utils'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipPopup, TooltipTrigger } from '@/components/ui/tooltip'
import App from './App'
```

## Localization

- The UI is in English; more locales come later. Every user-visible string goes through Paraglide: add it to `apps/desktop/messages/en.json` and render `m.<key>()` from `@/paraglide/messages`. Never hard-code UI text in a component.
- Keys are `snake_case` and describe the meaning, not the screen (`no_project_selected`, not `sidebar_text_1`).
- Rust never writes UI text of its own (labels, errors, statuses). It sends codes and data; the UI picks the message. Agent output passes through untranslated.
- Before localization work, read `apps/desktop/project.inlang/README.md` (git-ignored, so search tools may skip it). Never edit `src/paraglide/`.
- The locale is resolved from `localStorage`, then the OS language, then English (`project.inlang/paraglide.config.ts`).

## Styling

- Use `cn` for conditional or merged classes. Never build class strings with template literals.
- Biome sorts Tailwind classes (`useTailwindSortedClasses`), including those inside `cn()` and `cva()`. Run `bun run fix` and let it order them.

## Tests

- **Rust:** unit tests for a module go in a `#[cfg(test)] mod tests` at the bottom of that file. Tests that use a crate only through its public API go in that crate's `tests/` folder.
- **TypeScript:** pure logic has unit tests in `apps/desktop/tests/`, mirroring `src` (`tests/lib/format.test.ts` tests `src/lib/format.ts`), never inside `src`. They import from `bun:test`.
- Test behavior that can break (orchestrator decisions, provider protocol parsing, persistence). Skip tests for what types already guarantee.

## Git

- Only commit when the developer asks.
- Split commits only when the changes are unrelated.
- Write messages in English with Conventional Commits prefixes (`feat`, `fix`, `chore`…). Use the `caveman:caveman-commit` skill when available.
- No attribution in commits or pull requests: no co-author trailer, no "Generated with" line, even if a tool adds one by default.
- Never push unless asked.
