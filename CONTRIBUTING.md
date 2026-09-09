# Contributing to DB Free

Thanks for your interest in contributing to DB Free — a lightweight, native
database workbench (Tauri v2 Rust core + React/TypeScript UI, 69 engines,
offline-first, no telemetry).

Please read the [Code of Conduct](./CODE_OF_CONDUCT.md) first. By participating,
you agree to abide by it.

## Quick start

Requirements: Rust (stable), Node 22, pnpm 11. On Linux also:

```sh
sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev \
  librsvg2-dev patchelf libdbus-1-dev pkg-config libclang-dev clang cmake
```

```sh
pnpm install
pnpm tauri dev        # desktop app
pnpm check            # guardrail + tsc + eslint + clippy + cargo test
```

Architecture and rules: see [`CLAUDE.md`](./CLAUDE.md). The short version:

* UI calls Rust only through `src/lib/ipc.ts` (`invoke` + `CommandMap`).
* Every `#[tauri::command]` passes through `guard::` (`src-tauri/src/guard/mod.rs`).
* Services orchestrate; per-engine logic lives in `src-tauri/src/integrations/`
  (one file per adapter family). App state lives in `store/` (SQLite).
* Every `.rs` / `.ts(x)` file starts with a `SOT:` line.
* TS types come from `src/lib/bindings` — regenerate with `pnpm bindings`
  after changing `#[ts(export)]` Rust types, never hand-write them.
* UI is HeroUI v3 compound components with `onPress`, on the black theme.
  Use semantic color utilities (`bg-background`, `bg-surface`,
  `text-foreground`, `text-muted`, `border-border`, `bg-accent`) — never raw
  palette classes.
* No `any`, `as` casts, or `@ts-ignore`; `unknown` only in `src/lib/ipc.ts`.
  Rust: no `unwrap` / `expect` / `panic` outside tests (clippy denies them).
* Secrets are never returned to the UI, are sealed with AES-256-GCM, and the
  key lives in the OS keychain.

## How to contribute

1. **Find or open an issue.** For bugs include the template fields
   (version, OS, engine, repro steps). For a new engine, use the engine-request
   template — wire-compatible products (Valkey, ScyllaDB, OpenSearch, …) are
   usually just a new `Engine` variant on an existing family.
2. **Fork and branch.** Branch from `main`:
   `feat/<short-name>`, `fix/<short-name>`, or `docs/<short-name>`.
3. **Make the change.** Keep it focused; one concern per PR.
   * Grep before creating: `grep -rl "SOT:.*<keyword>" src src-tauri/src`.
   * UI component? Read the HeroUI v3 docs first
     (`.claude/skills/heroui-react/scripts/get_component_docs.mjs <Name>`).
   * Engine / ObjectKind / Tool / command? Follow the numbered recipes in
     `src-tauri/src/integrations/mod.rs`, `src/lib/objects.ts`,
     and `src-tauri/src/commands/mod.rs`.
4. **Verify.** Run `pnpm check` (guardrail, tsc, eslint, clippy, cargo test)
   and hand back green. Add or update tests for the behavior you changed.
   Live adapter tests are gated on env vars — see `./scripts/live-tests.sh`.
5. **Open a PR** against `main` using the PR template. Link the issue,
   describe what/why/how, and note the `pnpm check` result.

## Commit messages

We use [Conventional Commits](https://www.conventionalcommits.org/). The
release workflow derives the next semver from commit subjects since the last
tag, so write them carefully:

* `feat(scope): ...` — minor bump (new feature)
* `fix(scope): ...` — patch bump (bug fix)
* `feat!: ...` or `fix!: ...` (or `BREAKING CHANGE:` footer) — major bump
* `docs:`, `chore:`, `refactor:`, `test:` — no version bump

Examples: `feat(export): download query results as CSV`,
`fix(sqlite): browse views without rowid ordering`.

## Reporting bugs / requesting features

Use the issue templates (bug report, feature request, engine request).
Good reports include: DB Free version (or commit), OS + arch, engine + server
version, minimal repro steps or query, expected vs actual behavior, and logs
(run with `RUST_LOG=debug` where relevant). Never paste secrets, connection
strings with passwords, or private data.

## Security

Do not open public issues for vulnerabilities. See
[`SECURITY.md`](./SECURITY.md) for how to report them privately.

## License

Contributions are licensed under the [MIT License](./LICENSE). By submitting a
PR you agree your changes are provided under the same license.
