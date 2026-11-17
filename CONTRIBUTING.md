# Contributing

hoshi is built by a small team. Reviewing a pull request often takes more time than writing it, so
read this before you open one.

## What we accept

- **Bug fixes.** A crash, a regression, a real performance problem, something that does not work
  as it should. Always welcome.
- **Features, only when agreed first.** Open an idea in
  [Discussions](https://github.com/nxtvoid/hoshi/discussions/categories/ideas) and wait for a
  maintainer to say yes before you write code. A feature pull request without that is closed.

Not wanted unless a maintainer asks: UI redesigns, architecture rewrites, new platforms, and
cleanup or refactor pull requests on their own.

## Other ways to help

- **Report bugs** with the issue form. A good repro is worth more than a fix.
- **Close the loop:** comment when an issue is a duplicate, or no longer happens on the latest
  release (say which version).
- **Test open pull requests** and say what you saw. That is better than a second pull request for
  the same bug.

## Rules for a pull request

- **One problem per pull request.** Include everything that fix needs (core, providers, UI, tests),
  nothing else. A second fix or a refactor goes in its own pull request.
- **Show that it works.** Say how you found the bug, how you checked the fix and what you saw.
  "Tests pass" is not enough. UI changes need before and after screenshots; add a short recording
  when the change is about motion or timing. Upload them to the pull request, do not commit them.
- **Keep it green and mergeable.** CI runs the format, lint and test commands below. Rebase when
  `main` moves.
- **Title in Conventional Commits,** in plain language: `fix(composer): pasted images keep their order`.
- **Agents are fine.** hoshi is built with them. Review what your agent wrote as if you wrote it,
  and say in the pull request which agent and model did the work.

A pull request that follows the rules can still be closed. We do not owe a review or a merge.

## Setup

hoshi is Rust. You need:

- [Rust](https://rustup.rs) (stable; `rust-toolchain.toml` picks it and its components).
- On Linux, the libraries GPUI builds against. On Debian or Ubuntu:

  ```sh
  sudo apt-get install libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev \
    libfontconfig1-dev libfreetype6-dev libdbus-1-dev pkg-config
  ```

- At least one agent CLI signed in (Claude Code, Codex or GitHub Copilot) to try threads end to end.

Then:

```sh
cargo run -p hoshi-native                                # start the app (debug build)
cargo fmt --all                                          # format
cargo clippy --workspace --all-targets -- -D warnings    # lint; any warning fails CI
cargo test --workspace                                   # test
```

[Bun](https://bun.sh) is optional. It runs [Biome](https://biomejs.dev), which formats and lints
the JSON and CSS files (such as `apps/native/messages/en.json` and
`crates/hoshi-ui/assets/globals.css`), and the release scripts in `scripts/`. If you change those
files, run `bun i` once and then `bun run check` (Biome, rustfmt and clippy) or `bun run fix` to
format. Without Bun, CI still catches what Biome would. `bun run dev` and `bun run test` are
shortcuts for the cargo commands above, plus the scripts' tests.

A debug build keeps its data in `~/.hoshi-dev`, never in the installed app's `~/.hoshi`. Set
`HOSHI_HOME` to use another folder, for example a fresh one to reproduce a bug from a clean start.

## Where things live

[`AGENTS.md`](AGENTS.md) is the full guide: the layout of the workspace, the architecture rules,
code conventions, localization, styling and tests. Your agent reads it on its own; read it
yourself before a larger change. The short version:

- `crates/hoshi-core` is pure: commands in, events out, no I/O.
- Provider specifics stay in their adapter in `crates/hoshi-providers`.
- `apps/native` renders state and holds no business logic.
- Every user-visible string goes through `apps/native/messages/en.json`; every color is a theme
  token.
- No real personal data in code, tests or pull requests: invent names, paths and repositories.

## Security

Do not open a public issue for a vulnerability. See [SECURITY.md](.github/SECURITY.md).
