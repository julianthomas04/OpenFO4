# Legal provenance

This ledger records known source boundaries for release review. It does not grant rights, replace third-party notices, or certify a release as cleared. The release process must fail closed when a required license, attribution, or corresponding-source item is unknown or missing.

| Material | Provenance and distribution boundary |
|---|---|
| Original OpenFO4 engine and installer source, scripts, tests, and documentation | Independently authored project material, licensed Apache-2.0 except where a component/file has another license. Copyright 2026 Julian Thomas and OpenFO4 contributors; contributors retain copyright, with no assignment. |
| Pip-Boy behavior routines in Rust | Maintainer attestation dated 2026-10-18: clean-room functional reimplementations based on observed runtime behavior and constants extracted from an installed game. The attested routines include the Pip-Boy header, status page, perk colour filter, and shared text-setting rule. They do not copy the corresponding ActionScript routines. This attestation applies only to those routines. |
| Fallout 4 assets and runtime-extracted game data | Supplied by the user's local, legally obtained installation. Bethesda-owned assets are not included in the repository or distributed by OpenFO4. Runtime use does not grant redistribution rights. |
| Third-party libraries, tools, and fonts | Retain their own upstream licenses, copyrights, attribution, and copyleft requirements. See [Licensing and provenance](licensing.md) and the release-specific generated notices and source inventory. |
| Combined installer executable | GPL-3.0 distribution terms apply because it links 18 GPL-3.0 libraries. This does not change the license of independently authored installer source. |
| Separate Bink2 decoder and Librempeg | Bink2 decoder retains GPL-3.0-or-later with LGPL-2.1-or-later portions and port credits. Librempeg remains a separate AGPL-3.0-or-later tool. |

No broader clean-room claim is made for imported libraries, decoder implementations, upstream code, or any other material not explicitly listed above. A release remains blocked if its actual dependency set or corresponding source cannot be accounted for by the release inventory.
