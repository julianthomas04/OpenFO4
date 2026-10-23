# BA2Sharp provenance

The retained BA2Sharp library code derives from Alexander Ellingsen's GPL version 3 BSA Browser / Sharp.BSA.BA2 project: <https://github.com/AlexxEG/BSA_Browser>. Copyright 2016-2023 Alexander Ellingsen. The source review baseline is commit `a2ce9d89ba39cccef8f9203414e78d8e5174647c`; this is not claimed as the exact original import commit. The full GPL text is bundled in the release component notices.

OpenFO4 contributors modified the retained files in 2026 for nullable annotations, defensive error handling, streaming and buffer reads, culture-independent strings, property refactoring and BA2/texture handling. Some files have only updated provenance notices. Each retained C# file carries a modification notice. The complete current preferred source and installer build metadata are in the project source archive; a separately pinned selected upstream baseline is provided for provenance.

`BA2Util/DDS.cs` is an independent Apache-2.0 OpenFO4 implementation of published DDS/DXGI format facts. It is excluded from this component's GPL source selection. The earlier BA2Lib-derived declarations and the upstream baseline DDS.cs are not redistributed. This exception does not relicense the other BA2Sharp files.

Upstream README credits include FOMM contributors Timeslip, Q and kaburke, plus bsaopt, F4SE and BA2Lib references. The complete upstream README is retained as attribution evidence. A reference to a project is not a grant to redistribute code under that project's separate terms.
