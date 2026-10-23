# Development

Building, testing, linting and CI for the two code bases: the Rust engine (`src/engine`) and the C# installer (`src/installer`, tests in `tests/installer.Tests`). Windows is the primary platform.

Quick start: `pwsh ci.ps1` runs everything CI runs, in the same order, and never starts the game or the installer.

## Prerequisites

| Tool | Version | Source of truth |
|---|---|---|
| Rust | stable, with `rustfmt` and `clippy` | `src/engine/rust-toolchain.toml` |
| .NET SDK | 8.0.400 or newer feature band | `global.json` |
| ReportGenerator | 5.5.11 (dotnet local tool) | `.config/dotnet-tools.json`, restored by `dotnet tool restore` |
| cargo-llvm-cov | latest (needs `rustup component add llvm-tools-preview`) | `cargo install cargo-llvm-cov --locked` |
| cargo-deny, cargo-machete | latest | `cargo install cargo-deny cargo-machete --locked` |
| naga-cli | the `naga` version in `src/engine/Cargo.lock`, printed by `pwsh tools/naga_version.ps1` | `cargo install naga-cli --version <that version> --locked` |

The cargo tools are only needed for coverage and the dependency checks. naga-cli is the judge of the C# shader-translator tests, which feed the emitted WGSL to the same compiler the engine validates with. `ci.ps1` requires it, at exactly the locked version, unless you pass `-SkipNaga`.

## Building

Engine (from `src/engine`):

```powershell
cargo build            # dev profile, optimized with debuginfo
```

The engine finds its install (the folder holding `assets` and `cache`) through `OPENFO4_INSTALL_DIR`, else next to the executable; see [../install-contract.md](../install-contract.md) for the install layout. A development build in `target` has no install beside it, so set `OPENFO4_INSTALL_DIR` to your converted install before `cargo run` and before the real-install tests (`-- --ignored`). Release builds use fat LTO and are slow; do not build them casually.

Installer (from the repo root):

```powershell
dotnet build OpenFO4.sln -c Release
```

Use Release for anything performance sensitive; see [../installer/README.md](../installer/README.md).

Set `CARGO_INCREMENTAL=0` if the linker reports phantom `LNK2019 ... anon.llvm` unresolved symbols (corrupted incremental cache). `ci.ps1` sets it already.

## Logs and console

Every run writes `<install dir>/logs/openfo4-YYYY-MM-DD_HH-MM-SS.log` (local session start, no ANSI colours, local-time timestamps with milliseconds per line); for dev runs that is the folder in `OPENFO4_INSTALL_DIR`. The 10 newest session logs are kept. Dev (debug) builds also log to the console; release builds use the Windows GUI subsystem and have no console, so the file is the only log. `RUST_LOG` applies to both sinks. Panics are logged with location and backtrace and flushed before the process aborts. If the log file cannot be created, the game keeps running without it: stderr gets a warning and an error toast appears in game. Details: [debugging](../engine/debugging.md#logging).

## Running tests

### Rust

```powershell
cd src/engine
cargo test --locked                    # unit tests, tests/ integration tests, doctests
cargo test --locked -- --ignored       # additionally the GPU and real-install tests (GPU adapter, OPENFO4_INSTALL_DIR)
```

- Unit tests live in `#[cfg(test)] mod tests` at the bottom of the file under test; cross-module tests live in `src/engine/tests/`.
- Tests marked `#[ignore = "needs a GPU adapter"]` need a real adapter and are skipped in CI. Run them locally with `--ignored` or `ci.ps1 -IncludeIgnored`.
- `tests/db_accessors.rs` loads the real schema (`src/installer/file_converters/esm_schema.sql`) into an in-memory SQLite database, so schema and accessor changes are checked together.
- Real-game-data tests, if added, must be `#[ignore = "needs FO4 data"]` and skip cleanly when the data is absent.

### C#

```powershell
dotnet test OpenFO4.sln -c Release
```

xUnit v3, tests mirror the source folders. Tests that need real game data must carry `[Trait("Category", "RealData")]` and skip when the data path is absent; run or exclude them with `--filter "Category=RealData"` / `--filter "Category!=RealData"`. No such tests exist yet, and CI never has game data.

The WGSL validation tests (`NagaValidationTests`) run the naga executable whose file path is in `OPENFO4_NAGA`; a bare command name is not looked up on `PATH`. Locally they skip while the variable is unset, but a variable that does not point to an existing file fails them, locally too. On GitHub Actions an unset variable fails them as well, so CI cannot silently drop the check. When the variable is unset, `ci.ps1` exports it from the `naga` on `PATH`; with `-SkipNaga` it does not, yet a preset `OPENFO4_NAGA` still runs the tests.

## Lint and format

Warnings are errors in CI. Fix locally before pushing.

| Language | Check | Fix |
|---|---|---|
| Rust format | `cargo fmt --all -- --check` | `cargo fmt --all` |
| Rust lint | `cargo clippy --all-targets --locked -- -D warnings` (lint levels in `[lints]` of `src/engine/Cargo.toml`) | fix the code; an `#[allow]` needs a reason |
| C# format | `dotnet format OpenFO4.sln --verify-no-changes` | `dotnet format OpenFO4.sln` |
| C# build | `dotnet build OpenFO4.sln -c Release -warnaserror` (analyzers via `Directory.Build.props`, severities in `.editorconfig`) | fix the warning |

## CI

`.github/workflows/ci.yml` runs on every push and pull request; a newer push on the same ref cancels the older run. Permissions are read-only. The same steps run locally through `ci.ps1`.

| Job | Runner | Checks |
|---|---|---|
| `repo-guard` | ubuntu | `python tools/repo_guard`: no game-derived material in the tree (see below) |
| `rust-fmt` | ubuntu | `cargo fmt --check` |
| `rust-lint` | windows | `cargo clippy --all-targets --locked -D warnings` |
| `rust-test` | windows | all tests under `cargo llvm-cov`; coverage summary and `lcov.info` artifact |
| `rust-deps` | ubuntu | `cargo deny check` (advisories, licences, sources; `src/engine/deny.toml`) and `cargo machete` (unused dependencies) |
| `python` | ubuntu | `unittest` of the installer collision worker (`tests/installer_python`) and of the repo guard (`tests/repo_guard`) |
| `dotnet` | windows | restore, `dotnet format --verify-no-changes`, Release build with `-warnaserror`, naga-cli install, tests with coverage; ReportGenerator summary and Cobertura artifact |

Build caches (Rust and NuGet) are only saved on `main`, so other branches and pull requests restore `main`'s cache without adding their own copies. The Rust test cache is saved even when tests fail. The naga-cli binary (about 5 MB, built by `cargo install --locked` at the `naga` version in `Cargo.lock`) is the exception: it is saved on every ref under a key of that version, right after the build and a `--version` smoke check, so a failing test step keeps it but a broken binary is never cached, because a miss costs a several-minute build. The coverage build uses `debug = "line-tables-only"` (via `CARGO_PROFILE_DEV_DEBUG`) to shorten Windows linking and shrink the cache; local builds keep full debug info.

### ci.ps1

```powershell
pwsh ci.ps1                      # everything, fail-fast
pwsh ci.ps1 -Coverage            # tests with coverage, reports in coverage/
pwsh ci.ps1 -SkipDotnet          # Rust only
pwsh ci.ps1 -SkipRust            # C# only
pwsh ci.ps1 -SkipDeps            # without cargo-deny / cargo-machete
pwsh ci.ps1 -SkipNaga            # naga-cli not required (the WGSL validation tests skip unless OPENFO4_NAGA is preset)
pwsh ci.ps1 -SkipGuard           # without the repo guard and its tests
pwsh ci.ps1 -IncludeIgnored      # also the GPU tests
```


### Repo guard

`python tools/repo_guard` (Python 3.11, standard library only) scans every file git would commit (tracked plus untracked and not ignored) and fails when something looks like a game asset or installer output:

| Rule | Flags |
|---|---|
| `forbidden-extension` | game and converted-asset extensions (`.nif`, `.dds`, `.ba2`, `.esm`, `.pex`, `.hkx`, `.fxp`, `.glb`, `.ttf`, ...) |
| `forbidden-directory` | directories named `cache`, `translated`, `converted`, `extracted`, `unpacked` |
| `game-magic` | binary files starting with a game-format signature (DXBC, BA2, BSA, TES4, Gamebryo NIF, DDS, Havok packfile/tagfile, PEX, xWMA, FUZ, BGSM/BGEM, SWF/GFX) |
| `embedded-dxbc` | a valid DXBC container at a non-zero offset of any file (shader packages, archives) |
| `translated-wgsl` | text that carries the DXBC to WGSL emitter's vocabulary (`X_in`/`X_out` interface structs with `mask=0x` comments, `var rN: vec4<f32>` temporaries, its deferred-discard and float-bit helper variables, immediate-constant-buffer arrays) or eight or more distinct `rN` names |
| `dxbc-disassembly` | shader disassembly listings (`dcl_*` lines, `ps_5_0` headers, fxc banner) |
| `encoded-blob`, `hex-byte-table`, `numeric-table` | 128+ character base64/hex runs, 64+ hex bytes, 128+ consecutive numeric literals |
| `unexpected-binary`, `oversized-text`, `symlink`, `unreadable` | binary files, text files over 1 MiB, symbolic links, files that cannot be read |

Exceptions live in `tools/repo_guard/allowlist.toml`: each `[[allow]]` entry lists glob `paths`, the `rules` it waives and a mandatory `justification`. An entry (or one of its rules) that waives nothing fails the guard, so stale exceptions cannot hide a future file. Exit status: 0 clean, 1 findings, 2 the guard could not run (`--no-git` scans a plain directory, `--root` and `--allowlist` override the defaults). `tests/repo_guard` holds its unit tests.

### cargo-deny

Licences must be on the allow list in `deny.toml` (permissive licences only). Advisories in `[advisories] ignore` each carry a reason; remove the entry once the dependency is upgraded. Prefer `cargo update -p <crate>` over an ignore when a fix exists.

## Coverage

Coverage is reported, never gated: there is no target and a PR is not judged on the number. It is a map of untested areas, not a goal.

- Locally: `pwsh ci.ps1 -Coverage`. Rust output: `coverage/rust/lcov.info` and `summary.txt`; C# output: `coverage/dotnet/report/` (`SummaryGithub.md`, `Summary.txt`, `Cobertura.xml`). Open `lcov.info` in an editor extension such as Coverage Gutters.
- In CI: each coverage job writes a summary to the GitHub job summary and uploads the reports as the `rust-coverage` and `dotnet-coverage` artifacts. No external service is used.
- Rust coverage excludes doctests.

## Testing philosophy and conventions

The goal is valuable tests, not a coverage number.

**What to test**
- Code with branching or domain logic where a bug is plausible and would hurt: parsers and binary decoders, format converters, coordinate conversions, math (frustum, cascades, culling, animation sampling), streaming and LOD decisions, allocators and ring buffers, INI/env/config parsing, database accessors, physics configuration, deterministic simulation invariants (fixed dt, N steps, no NaN, bodies settle).
- Edge cases: empty, boundary, maximum, truncated or malformed input, out-of-range offsets, bad magic, zero counts. Error paths must produce the specific error, not panic.
- Properties (round-trip, inverse transforms, "never panics on arbitrary bytes") where they express the requirement better than examples. Rust property tests use `proptest` (a dev-dependency).

**What not to test**
- Trivial getters, constructors, DTOs, pass-through wrappers, constants.
- Mirror tests: the expected value must not be computed with the same formula as the code. Take it from a hand calculation, the format spec, a known reference value or a round-trip.
- Private call sequences or call counts. Assert observable behaviour: return values, written rows or files, state after a step.
- GPU or window code (no golden images). Anything that needs real game data in default CI.

**Rules**
- One behaviour per test, named as a sentence (Rust `parse_rejects_truncated_header`, C# `Method_Condition_Result`).
- Deterministic: no sleeps, no wall clock, no shared mutable global state, no order dependence. Pass `dt` instead of reading a clock. Use `tempfile` (Rust) or a unique disposed temp directory (C#).
- No logic in test bodies beyond iterating a table of cases. Readable beats clever; small fixture builders are fine.
- Test doubles: prefer the real thing or a prebuilt in-memory implementation (in-memory SQLite with the real schema, temp directories, Mutagen's in-memory mod objects). Hand-written fakes can silently diverge from the real component; write one only when no prebuilt option exists and say why in the PR. No mocking frameworks.
- Synthetic fixtures for binary formats are built inside the test by a small builder that follows the documented format. Never commit FO4 game data.
- Mutation-check habit: after writing a test, temporarily break the code under test (flip a condition, off-by-one) and confirm the test fails, then restore. Every test must be able to fail.
- A test that finds a bug is not "fixed" by editing the test.
- Testability refactors (splitting pure logic out of IO or GPU code, accepting a connection, reader or path instead of a hard-coded global) are welcome when minimal and behaviour-preserving.

## Dependency policy

External dependencies are encouraged where they save work, but every new dependency (crate, NuGet package, dotnet tool, GitHub Action) needs explicit maintainer approval. Licences must pass `cargo deny`; unused Rust dependencies fail `cargo machete`. Dependabot proposes weekly updates for cargo, NuGet and GitHub Actions.
