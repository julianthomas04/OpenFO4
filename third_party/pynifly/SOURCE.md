# Modified PyNifly helper

This directory contains a modified GPL-3.0 PyNifly helper from <https://github.com/BadDogSkyrim/PyNifly>, not original Apache-2.0 OpenFO4 code. The upstream project and its contributors retain their rights. See LICENSE for the full GNU GPL version 3.

The source review baseline is commit `db74b27e16d957fc1a709e3e9e571975cad66cdc` (2026-03-27), the last upstream helper revision before this repository imported it on 2026-04-24. This is a review baseline, not a claim that the original import exactly matched that commit. Upstream path: `io_scene_nifly/pyn/bhk_autounpack.py`.

OpenFO4 contributors modified this helper in 2026 for per-body collision properties and materials, ragdolls and constraints, compounds and capsule geometry, NIF node identification and transforms, and collision worker integration. The shipped preferred source is bhk_autounpack.py in this directory. It is interpreted by Python and has no separate compilation step. The release source asset includes this modified file, this notice, the GPL text, and the exact pinned upstream helper and project evidence. No game assets are included.
