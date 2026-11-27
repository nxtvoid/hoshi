# hoshi

## Layout

- `apps/native` - The app, drawn natively with GPUI (`gpui-kit`). Renders state; holds no business logic. Its binary is `hoshi`; its installers are configured under `[package.metadata.packager]` in its `Cargo.toml`.
  - `messages/en.json`, `messages/es.json` - UI text, turned by `build.rs` into one `m::<key>()` function per message that picks the locale shown (`src/locale.rs`).
  - `icons`, `linux` - App icons (release and `dev`) and the Linux desktop entry and metainfo the `.deb` installs.
- `apps/web` - The marketing site: Astro and Tailwind CSS v4, static, English only. Its colors are tokens copied from `crates/hoshi-ui/assets/globals.css`; its download links point at the latest GitHub release of `nxtvoid/hoshi`. Separate Bun project with its own lockfile.
- `crates/hoshi-ui` - Reusable native components and the theme (`assets/globals.css`, read by `theme.rs`). Components here know nothing about the app: no engine, no `hoshi-core` types, no stores; text and state come in from the caller.
- `crates/hoshi-app` - The home folder, settings and engine start.
- `crates/hoshi-core` - Domain model and orchestrator. Pure: commands in, events out. No I/O.
  Its types are the contract with the UI.
- `crates/hoshi-engine` - Runtime that wires core, store and providers: persists events, runs effects.
- `crates/hoshi-store` - SQLite persistence for events and projections, in `state.sqlite` under hoshi's home. Each append writes events and projection rows in one transaction.
- `crates/hoshi-providers` - One adapter per agent CLI behind a common trait: Claude Code, Codex and Copilot.
- `crates/hoshi-forge` - Forge adapters (GitHub, Bitbucket Cloud) for the pull requests of threads' branches.
- `vendor/gpui-base`, `vendor/markdown` - `gpui-base` and `markdown` with patches; see `vendor/README.md`.
- `scripts` - Release helpers run with Bun (`bump.ts`, `release-notes.ts`).

## Data

Everything hoshi writes lives in one folder, its home: `~/.hoshi`, or `~/.hoshi-dev` for debug builds (`bun run dev`) so they never touch the installed app's data. `HOSHI_HOME` overrides both. It holds `settings.json`, `state.sqlite`, `worktrees/`, `chats/`, `attachments/`, `agent-processes/` (the agent processes each run started, so the next start ends what a crashed run left) and `caches/` (what the provider CLIs last listed, shown at start before they answer).

## Commands

- `bun i` installs the JS tools: Biome, and the Tauri CLI the release workflow signs installers with. `bun run dev` starts the app. `bun run build` builds it in release.
- `bun run check` runs Biome, rustfmt and clippy. `bun run fix` applies formatting.
- `bun run test` runs the Rust tests and the scripts' tests.
- `bun run web:dev` and `bun run web:build` install the site's dependencies and start or build it (output in `apps/web/dist`).
- `bun run bump <version>` sets the version in `Cargo.toml`; a `v<version>` tag builds the installers (`.github/workflows/release.yml`).
- Installers locally: `cargo install cargo-packager --locked`, then `cargo packager --release -p hoshi-native -f nsis` (Windows) or `-f deb` (Linux).

### Not decided yet — ask before establishing a pattern

The error type for library crates.

## CodeGraph

The repo is indexed by [CodeGraph](https://github.com/colbymchenry/codegraph). Only `.codegraph/.gitignore` is committed; the index (`.codegraph/codegraph.db`) is local to each machine. Set it up once per machine with `npm i -g @colbymchenry/codegraph` and `codegraph init -y` at the repo root; while an agent runs its MCP server, a file watcher keeps the index in sync. To understand or locate code, query it before grepping or reading files: the `codegraph_explore` MCP tool, or `codegraph explore "<symbols or question>"` in a shell.

## Architecture rules

- Complexity belongs in the provider adapters. The orchestrator stays pure and the UI stays dumb.
- Adding a provider means implementing the adapter trait; nothing else should change.
- A thread runs in its own working directory. A thread on a worktree spawns its agent
  with that worktree as `cwd`, never the project root.
- Performance matters: long sessions and many parallel sessions must stay smooth.
  Virtualize long lists and load transcripts a page at a time, never whole.
- Types that are persisted or written to JSON use camelCase:
  `rename_all = "camelCase"` on structs; on enums also `rename_all_fields = "camelCase"`.

## Code conventions

- **Identifiers in English:** types, functions, variables, enum values, file names.
- **No real personal data:** never copy names, usernames, emails, paths, repositories, branches, ticket ids or text from the developer's screenshots, threads or machine into code, tests, fixtures, commits or pull requests, not even to reproduce a bug. Invent them (`Test User`, `acme/shop`, `/home/dev/project`) or use `nxtvoid`.
- **No narrating comments:** do not write comments that explain why the code does something or restate a rule. Keep a short comment only for genuinely non-obvious intent.
- **Rust** follows rustfmt and clippy; `bun run check` fails on any clippy warning. **TypeScript** (the scripts) follows `biome.json` (single quotes, no semicolons).

## Localization

- The UI is in English and Spanish. Every user-visible string goes through `apps/native/messages`: add the key to every locale's file, with the same parameters (the build fails otherwise), and render `m::<key>()`. Never hard-code UI text in a component.
- Text kept outside a render (a placeholder, a label set once) follows a language change through `locale::input`, `locale::textarea` or `LocaleChanged`. Dates and times go through `moment.rs`, joined lists through `m::list_of_two` and `m::list_of_many`.
- Keys are `snake_case` and describe the meaning, not the screen (`no_project_selected`, not `sidebar_text_1`).
- Rust outside the app never writes UI text of its own (labels, errors, statuses). It sends codes and data; the app picks the message. Agent output passes through untranslated.

## Styling

- **Themes:** light, dark and system are supported, plus imported themes. Every color is a token in `crates/hoshi-ui/assets/globals.css`, defined for `:root` (light) and `.dark`, and read into `hoshi_ui::theme::Colors`. Components use those tokens (`colors.sidebar`, `colors.muted_foreground`), never literal colors. A new color is a new token in both themes.

## Tests

- **Rust:** unit tests for a module go in a `#[cfg(test)] mod tests` at the bottom of that file; once they outgrow the code beside them (a few hundred lines), in their own file as `#[cfg(test)] mod tests;` (`foo.rs` → `foo/tests.rs`). Tests that use a crate only through its public API go in that crate's `tests/` folder. UI behavior is tested headless with `#[gpui_kit::test]`.
- **TypeScript:** the scripts' tests live in `scripts/tests` and import from `bun:test`.
- Test behavior that can break (orchestrator decisions, provider protocol parsing, persistence). Skip tests for what types already guarantee.
- **What agents read:** `agent-context-budget.json` holds the exact size of the instructions and MCP tool descriptions hoshi gives agents; a change to them, up or down, updates it. Before and after such a change, run the cases it touches in `crates/hoshi-engine/tests/tool_use_evals.rs` (real Claude Code, spends tokens: `cargo test -p hoshi-engine --test tool_use_evals -- --ignored`). A new case reproduces a real mistake and fails on the text before the fix.

## Git

- Only commit when the developer asks.
- Split commits only when the changes are unrelated.
- Write messages in English with Conventional Commits prefixes (`feat`, `fix`, `chore`…). Use the `caveman:caveman-commit` skill when available.
- No attribution in commits or pull requests: no co-author trailer, no "Generated with" line, even if a tool adds one by default.
- Never push unless asked.
