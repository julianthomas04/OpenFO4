# Third-party source notices

The combined standalone decoder is distributed under GPL-3.0-or-later. See LICENSE.

The Bink2 G algorithms and tables derive from the pinned source identified in DECODER_SOURCES.md. That source retains the following copyrights and LGPL-2.1-or-later terms:

- Copyright (c) 2014 Konstantin Shishkov
- Copyright (c) 2019 Paul B Mahol

The retained full notice appears in Bink2Postprocess.cs. A copy of LGPL-2.1 is supplied as COPYING.LGPLv2.1. Source-specific LGPL notices do not change the GPL-3.0-or-later terms of the combined decoder.

The post-processing diverges from that pinned source in five places, listed in README.md. Those changes were derived empirically by black-box comparison against output frames produced by RAD's tools, not from any RAD source code or binary, and no RAD code is included.

Frame dispatch/context derives from Librempeg's GPL-3.0-or-later Bink2 decoder by the same copyright holders. The container layout follows its GPL-3.0-or-later Bink demuxer, copyright (c) 2008-2010 Peter Ross and copyright (c) 2009 Daniel Verkamp. Bit access, file validation, CLI handling, output packing and synthetic self-tests are implemented in managed code using the .NET standard library.

This directory contains no original game media, extracted asset data, proprietary runtime, or reference-image fixtures. Reference frames used for comparison are external to the repository and are not distributed.
