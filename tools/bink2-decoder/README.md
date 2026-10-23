# Managed Bink2 decoder

Standalone .NET 8 executable, using only the standard library. The production installer invokes it as a separate process. The combined source is GPL-3.0-or-later; see LICENSE, THIRD_PARTY_NOTICES.md and DECODER_SOURCES.md for retained notices, source build instructions and pinned provenance.

The supported profile is deliberately narrow: KB2g, video flags 0x10, no alpha, width aligned to 32 from 64 through 4096, even height through 4096, and frame feature words 0xE000, 0x2018, 0x20FF or 0xE018. Audio tracks are read from the header (flags limited to 16-bit, stereo and DCT) and their per-frame packets are skipped; audio is not decoded. Unsupported features fail explicitly.

## Divergences from the pinned algorithm

The base algorithm is the pinned LGPL patch listed in DECODER_SOURCES.md. It reproduces that source's decoder faithfully, including its image errors, so it is not a correctness reference. The post-processing now diverges from it in five places:

1. Edges between blocks inside a macroblock that are both inter-coded are filtered in the residual domain at decode time: the filter step is measured on the unclipped residual, and the taps are applied to the residual before it is added to the motion-compensated prediction.
2. Where two neighboring DC-only blocks differ by more than a measured step limit, the edge is filtered with the motion-edge strength instead of their own strengths.
3. The inter residual is rebuilt without intermediate clipping and clipped once, after the residual filtering, when it is added to the prediction.
4. DC-only reconstruction covers the whole coded picture, including coded rows and columns below or beyond the visible size.
5. Edges at or below the visible height are filtered like all other coded edges.

These behaviors were derived empirically by black-box comparison against frames produced by RAD's tools on the maintainer's machine. They were not taken from any RAD source code or binary.

Verification status: against those reference frames, the decoder is bit-exact on 284 sampled frames (frames 0 to 4026) in the R and B channels of a fitted RGB conversion model. Frames outside the sample are unverified. The reference frames are not distributed with this project.

## Commands

Build with the repository's existing .NET SDK. Run the resulting executable with:

```text
--self-test
info INPUT
decode INPUT --frames N --output OUTPUT.yuv
decode INPUT --start-frame N --frames COUNT --hash-output OUTPUT.jsonl
decode INPUT --frames N --output -
```

`info` prints width, height, frame count, frame rate and each audio track's id, sample rate and channel count as one line of camelCase JSON.

Output paths must not exist. The special `--output -` writes binary visible YUV bytes to stdout; in that mode both progress and the final JSON summary go to stderr. No text is mixed into binary stdout. File-output and hash-only modes continue to print their summary to stdout. Input media, reference data, manifests and generated output must stay outside the repository. No original assets or proprietary reference-tool dependencies belong in this project or CI.

Decoding always starts at frame zero to establish prediction history. `--start-frame` selects the first emitted frame; it does not seek past earlier decoding. `--frames` is the number emitted. Raw output contains visible Y, U, V planes in that order, with coded padding omitted, full range and no color conversion. Hash-only mode writes bounded JSONL containing zero-based source frame index and SHA256 of each visible plane plus their concatenation. Both output options may be used together. No raw output is retained in hash-only mode.

## Performance fields

The successful decode command prints a final JSON summary:

| Field | Meaning |
|---|---|
| `decodedFrames` | All frames decoded, including those before the selected output range. |
| `writtenFrames` | Selected frames packed and/or hashed. |
| `configuration` | Debug or Release compilation. Debug throughput is not representative of Release. |
| `elapsedMilliseconds` | Wall time from decoder construction through sequential packet reads, decoding, visible output packing, SHA256 hashing, progress reporting and final output-stream flushes. Excludes container opening/header validation, option parsing, output-file creation, summary serialization and process startup. Flush is not a durable-disk guarantee. |
| `decodedFramesPerSecond` | `decodedFrames` divided by that measured wall time. Output selection and storage mode therefore affect the result. |
| `managedAllocatedBytes` | Process-wide managed allocation counter delta over the same work, including transient allocations; not retained heap size or native allocation. No forced collection occurs. |
| `gcCollections` | Generation 0, 1 and 2 collection-count deltas over the work. |
| `processPeakWorkingSetBytes` | OS-reported process lifetime peak working set at completion; includes startup, JIT and earlier activity, rather than only the measured decode interval. |

Hashing is performed for selected frames even in raw-only mode. Compare runs only with matching build configuration, selected range and output options. First-run JIT, filesystem cache and storage load can affect timings. These measurements characterize this decoder; they do not establish engine playback performance or real-time presentation.

Synthetic self-tests require no media or test framework. Real-video parity checks use external raw-plane oracles; no such data is bundled here.

## Measured observations

One Windows Release run of an earlier revision, before the divergences above, decoded and hashed all 4,028 target frames in 76.600 seconds (52.585 fps): 82,616,320 bytes peak working set, 352,526,040 managed bytes allocated, GC counts 25/12/8. A separate bounded-pipe run automatically encoded managed output into a validated 4,028-frame H.264 MP4 in 106.953 seconds, including probe decoding/counting. No full raw-video dump was stored. These single-machine results establish narrow-profile pipeline feasibility, not general Bink2 support, production performance or engine integration.
