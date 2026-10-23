# Decoder source derivation

This standalone decoder ports the scalar Bink2 G algorithms into C#. The production installer invokes it as a separate process. It contains no source game assets and calls no native Bink decoder. The combined decoder is GPL-3.0-or-later; retained source-specific notices are recorded in THIRD_PARTY_NOTICES.md.

Pinned algorithm source: https://github.com/sharpemu/ffmpeg-core/blob/ce81eeef27fa0663bf51a22486494330852d69aa/bink2.patch (SHA256 2c1b70354bb32471a5b1bb46ca3479e641fdd9d971c3eb4317c935040dd28fdc). Its G source declares LGPL-2.1-or-later and copyrights Konstantin Shishkov (2014) and Paul B Mahol (2019). Context/frame dispatch is based on https://github.com/librempeg/librempeg/tree/395e8d99bed806c77b188f77599f25ee82f8f3bd, whose Bink2 files declare GPL-3.0-or-later and the same copyrights.

Tables were mechanically transcribed from the pinned G header, preserving values, ordering, dimensions and LE VLC codes. The scalar IDCT and DC/MV prediction equations preserve operation order and explicit 16-bit narrowing. Array/Span offsets replace C pointers. Frame and macroblock buffers are reused; the visible output remains full-range planar Y,U,V. Chroma algorithm component1 maps to output plane2 and component2 to plane1, matching the source.

The port was first validated for equivalence with the pinned algorithm, which retains known image errors. Post-processing was then corrected where it diverges from the pinned algorithm: residual-domain filtering of inner inter edges at decode time, a measured DC-only edge step limit, unclipped residual rebuild with one final clip, DC reconstruction over the whole coded picture, and filtering of edges at or below the visible height (details in README.md). These corrections were derived empirically by black-box comparison against output frames produced by RAD's tools on the maintainer's machine, not from any RAD source code or binary. Verification status: bit-exact on 284 sampled frames (0 to 4026) in the R and B channels of a fitted RGB conversion model.

Supported scope is narrow KB2g, silent, without alpha. Unsupported variants or frame features must fail explicitly. This executable is isolated from the installer and engine. External reference data and outputs must remain outside the repository.

## Building the distributed decoder

The matching OpenFO4-<version>-sources.zip release asset contains this directory, its GPL/LGPL texts, the pinned upstream algorithm patch, and the release build scripts. From the source root, install the .NET 8 SDK and run:

```powershell
dotnet publish tools/bink2-decoder/Bink2Decoder.csproj -c Release -r win-x64 --self-contained true -p:RuntimeFrameworkVersion=8.0.32 -p:PublishSingleFile=true -p:EnableCompressionInSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true -p:DebugType=none
```

The decoder has no NuGet package references. The self-contained .NET runtime retains its own binary license and third-party notices in the packaged runtime notices. Build SDKs are prerequisites; the source asset does not claim to contain their complete source closure.

Decoder safety limits: width must be at least64 and divisible by32; height must be even; each dimension is at most4096. Motion compensation validates its complete coded-plane read window, including fractional interpolation halos. Requests needing an unimplemented edge extension fail with frame/block/vector context rather than clamping or synthesizing pixels. In-bounds scalar arithmetic remains unchanged.
