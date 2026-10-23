# Licensing and provenance

OpenFO4 is independent software. This page describes license scope and distribution obligations; it is not legal advice or a warranty that every release is cleared for distribution. Release packaging must verify the actual dependency inventory, licenses, notices, and corresponding-source materials for that release.

## Scope

The repository's root [LICENSE](../LICENSE) applies Apache License 2.0 to independently authored OpenFO4 material, including engine and installer source, original scripts, tests, and documentation, except where a file or component states otherwise. Copyright is held by Julian Thomas and OpenFO4 contributors (2026). Contributors retain their own copyright; there is no contributor assignment.

This license does not grant rights to Fallout 4, its assets, trademarks, or other third-party material. OpenFO4 requires a legally obtained game installation. Bethesda-owned assets are read from that installation and are not included in this repository or distributed by OpenFO4. The project is not affiliated with or endorsed by Bethesda Softworks, ZeniMax Media, or Microsoft.

## Component exceptions and notices

| Component or distribution | Terms and scope |
|---|---|
| Independently authored engine and installer source | Apache-2.0, subject to the exceptions below. |
| Combined installer executable | Distributed under GPL-3.0 because it links 18 GPL-3.0 libraries. This does not relicense independently authored installer source, which remains Apache-2.0. The release must provide the applicable notices, license texts, and corresponding source required for this executable. |
| Separate Bink2 decoder | GPL-3.0-or-later; its port includes LGPL-2.1-or-later portions and the applicable port credits. Its obligations and attribution remain attached to that separate component. |
| Librempeg | AGPL-3.0-or-later, distributed as a separate standalone tool. |
| Rust dependencies | Six statically linked MPL-2.0 components are treated under their file-level copyleft terms; those terms permit combination with the otherwise Apache-2.0 engine. |
| Fonts | Each font retains its individual license and attribution. Consult the release's third-party notices and font license files. |

Do not infer a license for an individual dependency from this summary alone. The authoritative release inventory is generated from the dependency set for that artifact, and package-specific license texts and notices accompany the release. If inventory or source material is incomplete, distribution must be blocked until corrected.

The two MSVC 14.43.34808 static runtime components (`native:msvc-libcmt` and `native:msvc-libvcruntime`) are approved under a documented reviewed interpretation of the Visual Studio Community 2022 Section 4 application distribution clause read together with Microsoft's official [static-linking deployment guidance](https://learn.microsoft.com/en-us/cpp/windows/deployment-in-visual-cpp?view=msvc-170#static-linking). The guidance describes incorporating library object code in one application binary without distributing library files separately. The referenced REDIST list does not expressly enumerate these static archive paths; the class-level connection remains interpretive. This is not new Microsoft permission, an unambiguous express list match, or legal advice. Approval is limited to the exact archive identities and selected linker results recorded in `tools/licensing/evidence/microsoft-static-runtime-14.43.34808.json`; vendor notices and publisher/recipient distribution conditions remain applicable. It does not authorize redistribution of raw proprietary libraries or their source.

## Source and notices for releases

The installer is the prominent download on the [public GitHub Releases page](https://github.com/julianthomas04/OpenFO4/releases). Each release must also provide a clearly labeled source ZIP as a separate release asset, link it near the bottom of the release notes, and include the applicable notices and license texts. For a release tagged `<version>`, the source asset follows `OpenFO4-<version>-sources.zip`; its bundled component index is `legal/COMPONENT_INDEX.txt`, with machine-readable inventory at `legal/components.json`. A small public sidecar `OpenFO4-<version>-release-pair.json` pairs the version and source commit with the installer and source ZIP filenames, sizes, and SHA-256 hashes. Publish the installer, matching source ZIP, and pairing sidecar together on GitHub Releases. Releases use a clean tracked checkout and the normal locked Cargo and .NET builds. The source asset includes project source, build instructions, required third-party preferred sources, source modifications, and license texts; the component index records origin and modification information. These are naming templates; an asset link is valid only after that release asset is published and verified.

The canonical public repository is [github.com/julianthomas04/OpenFO4](https://github.com/julianthomas04/OpenFO4). Before each release, verify its actual release asset links, source ZIP, notices, and manifest. The source bundle is not a substitute for notices or corresponding source required with a binary.

## Provenance record

On 2026-10-18, the maintainer attested that the original Pip-Boy behavior routines identified below were independently reimplemented in clean-room Rust from observed behavior and runtime-extracted constants. The routines use no copied ActionScript source. This provenance statement is limited to those routines; it makes no claim about Bink2, upstream dependencies, fonts, or other imported material.

The relevant Rust comments now describe that implementation basis. Movie clip names and extracted constants identify interfaces and runtime data; they do not by themselves establish copied implementation code. Fallout 4 assets and constants generated or extracted at runtime remain sourced from the user's installation and are not distributed by this repository.

## Contributors

By submitting original code, scripts, tests, or documentation, a contributor makes that material available under Apache-2.0 while retaining copyright. No assignment of copyright is required. Contributors must disclose the origin and license of copied, adapted, generated, or third-party material and preserve upstream copyright and license notices. Do not submit material unless you have the right to contribute and redistribute it under its applicable terms.
