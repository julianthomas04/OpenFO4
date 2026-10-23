# Pinned Windows x64 video and audio tools

The project builds a separate Librempeg executable with libx264 and FLAC output. It does not link
the converter into the installer or engine. No Bethesda files belong in this
directory or in any repository build directory.

Run PowerShell 7 from the repository:

```powershell
& ./tools/librempeg/resolve.ps1
# Review and commit toolchain.lock.json before continuing.
& ./tools/librempeg/build.ps1 -Jobs 4
```

`resolve.ps1` is a one-time dependency acquisition operation. It downloads the
2026-09-27 MSYS2 base archive using the SHA256 from the official GitHub release
asset metadata, resolves the full additional package dependency closure through
MSYS2 pacman, verifies package signatures, and records exact package archive URLs
and SHA256 values. Base packages are pinned by the base archive. It also acquires
the two source archives at fixed full commit IDs. Their SHA256 values are recorded
at this first acquisition over HTTPS, not preclaimed as independently verified
upstream checksums. The x264 GitHub mirror is used because VideoLAN's web endpoint
can return a browser challenge instead of an archive.

The lock and cache are immutable inputs to `build.ps1`. It verifies all hashes,
extracts a fresh toolchain, installs only cached locked packages without refreshing
repository databases, and builds in a whitespace-free temporary directory. It
does not download or update dependency packages. The first MSYS2 login may refresh
its package-signing keys over the network; this does not resolve new dependencies.
To reproduce elsewhere, restore the exact
cache archives named in the lock. Preserve the complete cache with build artifacts;
package mirrors can eventually remove old files. Do not silently resolve a new
closure if an old archive disappears.

After an interrupted build, `build.ps1 -ResumeBuild` preserves the installed
toolchain, source trees and object files. It still verifies all cache hashes,
reruns configure and builds changed targets. A complete replacement bundle is
prepared separately; only after its manifest is written is it promoted, preserving
any previous bundle in a uniquely named backup directory. No build directories
are recursively removed by these scripts.

The output is `build/librempeg`. It contains ffmpeg.exe and ffprobe.exe, source
archives, license texts, build scripts, configuration and import reports, and a
SHA256 manifest. The initial static-link configuration rejects non-system imported
DLLs instead of silently omitting them. Review static runtime notices and the
import reports before redistribution. GCC runtime exception and MinGW runtime
license terms must be retained; the exception does not relicense project codecs.
Do not distribute a bundle until the source and license closure has been reviewed.

The build disables external-library autodetection, networking and nonfree
features. It explicitly enables AGPL, GPLv3 and libx264. It supports Bink/Bink2
input, the Bink audio tracks of the game's videos (binkaudio_dct, re-encoded with the native AAC encoder), H.264 MP4 output, silent raw/BMP diagnostics and H.264 decoding. BMP samples
avoid introducing a zlib dependency just for PNG output. For audio it reads xWMA
and WAV (WMA v2, PCM 8/16/24/32-bit and float, MS and IMA ADPCM) and writes FLAC (and decodes FLAC to verify output),
with the sample-format, channel-layout and sample-rate conversion filters ffmpeg
inserts automatically (`asf2sf`, `acl2cl`, `ardftsrc`; Librempeg has no
`aresample`) plus `aformat`, `anull` and `volumedetect`. Every enabled component is
asserted present in the generated configuration before Librempeg compilation in
`build.sh`; a missing one fails the build. x264 is built first because its installed
pkg-config metadata is required to configure Librempeg.

## Licence of the resulting binary

Librempeg's `ffmpeg` command line program only builds with `--enable-agpl`, so the
executable is licensed under the GNU AGPL version 3 or later. It also links x264
(GPL version 2 or later, used under GPL version 3), so the effective licence of the
bundle is AGPLv3 with the GPLv3 obligations of x264 and the complete corresponding
source in `source/`. The audio components carry no extra licence gate in configure
(no nonfree, no external libraries). The project distributes the converter as a
separate executable and does not link it into the installer or engine.
Both AGPLv3 and GPLv3 texts accompany the bundle. Retain corresponding-source
availability for recipients; if a modified version supports remote network
interaction, AGPL section 13 requires offering corresponding source to those
remote users. Disabling media network protocols does not remove license duties.

No bit-for-bit reproducibility claim is made until two builds are compared. Fixed
sources, packages and flags make acquisition reproducible; timestamps or compiler
metadata may still affect executable bytes.
