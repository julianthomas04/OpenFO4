# Contributing to OpenFO4

Thank you for your interest in contributing to OpenFO4!

OpenFO4 is an open-source project aiming to rebuild the Fallout 4 engine in Rust with a C# installer. Contributions of all kinds are welcome, whether you're an experienced Rust or C# developer or simply interested in helping the project grow.

Please read these guidelines before contributing.

## 1. Ways to Contribute

There are several ways to help improve OpenFO4:

- **Bug reports:** Identify and document bugs, crashes and unexpected behavior.
- **Code contributions:** Fix bugs, implement new features, optimize existing systems, or improve code quality.
- **Testing:** Test the engine across different hardware configurations and environments.
- **Documentation:** Improve technical documentation, setup instructions, and explanations.

You do not need to contribute code to make a valuable contribution.

## 2. Before You Start

Get the public repository with `git clone https://github.com/julianthomas04/OpenFO4.git`, then create a branch for your work. Issues and pull requests are tracked in the [OpenFO4 repository](https://github.com/julianthomas04/OpenFO4).

Before beginning work on a contribution:

1. Check existing issues and pull requests to avoid duplicate work.
2. For significant changes or new features, open an issue to discuss your proposal before implementation.
3. Keep contributions focused on a specific problem or improvement.
4. Consider how your changes fit into the project's existing architecture and long-term goals.

Small bug fixes, documentation improvements, and other straightforward changes generally do not require prior discussion.

## 3. Development Setup
### Prerequisites
- Windows 11. This project is designed to be developed on and for this OS.
- Rust 1.95.0 (stable toolchain, with `rustfmt` and `clippy`)
- .NET SDK 8.0.426
- Git
- PowerShell >= 7.0.0.
- An installed copy of Fallout 4

If you get a 'not digitally signed' error while running a PowerShell script, run this from the repo root:
```
Get-ChildItem -Recurse -File | Unblock-File
```
### Building the engine
Dev build (fewer compiler optimizations but faster build times):
```
cd src/engine
cargo build
```
You can find the build at `/src/engine/target/debug/openfo4.exe`.

Release build (maximum compiler optimizations but slow build times):
```
cd src/engine
cargo build --release
```
You can find the build at `/src/engine/target/release/openfo4.exe`.
### Building the installer
Run `build-release.ps1`. When asked, provide it with the project version you want to build for (for example `0.1.0-alpha`; letters, digits, `.`, `_` and `-`, starting with a letter or digit). The exact version you specify is not relevant in most cases in terms of functionality. You can find the built installer at `/build/release/OpenFO4-installer-<version>.exe`. Note that the script will not run if git detects uncommitted changes to avoid drifts between the published code and the released installer build. The script also builds the bundled FFDec and Librempeg tools; FFDec needs a JDK at `C:\Program Files\Java\jdk-23`, and the first Librempeg build compiles a full MSYS2 toolchain in `%TEMP%\openfo4-librempeg`, which takes a while.

### Running the CI pipeline
Run `ci.ps1`. This runs lint, typechecking and tests and some basic guards against tracked game assets. Despite these, you are ALWAYS responsible for keeping assets out of the codebase yourself. See [Legal Guidelines](#6-legal-guidelines).

## 4. Code Conventions

Follow the existing conventions and architectural patterns of the codebase. Avoid unrelated refactoring or introducing new dependencies without good reason.

Changes should be understandable, maintainable, and appropriately tested. More specific requirements may be communicated during code review.

## 5. Submitting Changes

1. Fork the repository and create a branch for your changes.
2. Implement your changes.
3. Check the project builds and the CI pipeline passes. Additionally manually verify your changes behave as expected. Ensure no Bethesda owned assets exist in your changes, as well as your git history. See [Legal Guidelines](#6-legal-guidelines).
4. Open a pull request against the `development` branch.

Your pull request should include:
- A clear description of the changes.
- The motivation or issue being addressed (including what issue this PR aims to address).
- Any relevant implementation details.
- How the changes were tested.
- Screenshots or recordings for visual changes, where applicable.

Please keep pull requests reasonably small and focused to make reviewing easier.

### Reviews

All pull requests are reviewed before merging.

Review feedback may include requests for changes, additional tests, or alternative implementations.

Submitting a pull request does not guarantee that it will be merged. Contributions may be declined due to technical concerns, scope, maintainability, or conflicts with the project's direction.

## 6. Legal Guidelines

OpenFO4 is an independent project and is not affiliated with or endorsed by Bethesda Softworks.

Original OpenFO4 code, documentation, scripts, and tests are offered under Apache-2.0; contributors keep copyright in their work and no contributor assignment is required. By submitting original material, you agree to make it available under Apache-2.0 and confirm you may do so. Identify any copied, adapted, generated, or otherwise third-party material and its source and license before proposing it. Third-party and derived files keep their own terms; see [docs/licensing.md](docs/licensing.md). Do not assume the repository's Apache-2.0 license overrides another component's license.

- Do not commit or upload proprietary Bethesda assets to the repository or its forks. This includes game data, textures, meshes, audio, and other proprietary content. This restriction applies to every commit introduced by your contribution, even if the assets have been deleted in subsequent commits. Removing an asset in a later commit does not remove it from Git history.
- Do not submit code copied from proprietary or otherwise incompatible sources.
- Only contribute material that you have the legal right to share under the project's license.
- Respect third-party licenses and attribution requirements.

This project follows a zero-tolerance policy for piracy. Pull requests containing proprietary Bethesda assets, including assets present only in the contribution's Git history, will be rejected and may be reported to GitHub for removal.

Repeated violations of this policy may result in a permanent ban from contributing to OpenFO4. Deliberate violations may result in an immediate ban without prior warning.

All contributors are responsible for ensuring they have the right to distribute the content they submit.

## 7. Community and Communication

For questions, discussions, and general development coordination, use the project's GitHub issues or Discord server.

Please communicate respectfully and keep technical discussions constructive.

Discord: https://discord.gg/tZgkngG5ZX

## 8. AI
Using large language models (LLMs) for part or all of a contribution is permitted.

Contributors are fully responsible for the correctness, quality, legality, and maintainability of everything they submit, regardless of whether AI was involved.

Contributors are expected to understand their submitted changes and be able to explain and maintain them during review.

Repeated violations of project guidelines or consistently submitting low-quality, unreviewed contributions may result in a ban from contributing to OpenFO4.

## 9. Final Notes

OpenFO4 is an evolving project, and these guidelines may change as its development and community grow.

Thank you for helping improve OpenFO4!
