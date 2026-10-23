# Minimal FFDec export tool

Copyright 2026 Julian Thomas and OpenFO4 contributors. Original files in this directory are Apache-2.0. Patched FFDec library sources retain their existing LGPL-3.0-or-later notices; the tool's use of GPL-2.0-or-later DoubleType code requires GPL-compatible combined distribution. No rights in upstream components are replaced by this directory's original-code license.

`prepare.ps1` downloads the pinned upstream 26.2.1 sources and creates preferred-form library sources in ignored `build/ffdec-work/src`. It excludes the application GUI, Adobe SWCs, native registry/video/WebP dependencies and the restricted Monte CMYK JPEG implementation. `build.ps1` uses the existing JDK's `javac --release 21` and `jar`; generated upstream lexer sources are retained.

The adapter implements the installer's font and SVG frame/sprite/shape exports. Unsupported options, visible unsupported SWF video and unsupported image formats fail with their cause. Unused video tags do not prevent font or unrelated frame exports. Standard JDK ImageIO replaces the restricted image reader.

Builds and conversion runs are scheduled by the orchestrator:

```powershell
pwsh -NoProfile -File tools/ffdec/build.ps1
```

Compiler diagnostics are in `build/ffdec-work/javac.log`. Runtime output is in `build/ffdec`; matching preferred-form sources, dependency sources, notices and offline build instructions are in `build/ffdec-sources`. `package.ps1` copies the tool and notices into the payload and the matching sources into the release source staging area. It does not require approval records or execution-proof metadata.

The functional export comparisons remain available for scheduled runs. `tests/contract.py` exercises original synthetic SWFs and compares actual font/SVG outputs with the reference tool, including malformed-input and unsupported-option behavior. `tests/required_inputs.py` compares the installer's real export jobs from a generated plan. These checks run the tools; they are not prerequisites for ordinary packaging and do not create approval records. Use fresh output directories for each comparison. The original synthetic fixtures and output normalization code remain in `tests/`.