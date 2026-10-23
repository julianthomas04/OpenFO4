// Copyright 2026 Julian Thomas and OpenFO4 contributors
// SPDX-License-Identifier: Apache-2.0
package com.jpexs.decompiler.flash.helpers;

import java.awt.image.BufferedImage;
import java.io.IOException;
import java.util.List;

/** Explicitly excludes optional native WebP conversion from the export tool. */
public final class UnsupportedWebP {
    public static final class Options {
        public static Options createLosslessConfig() { return new Options(); }
    }
    public static boolean isAvailable() { return false; }
    private static IOException unsupported() {
        IOException failure = new IOException("WebP is not supported by the OpenFO4 minimal exporter");
        openfo4.ExportDiagnostics.unsupported(failure);
        return failure;
    }
    public static byte[] encodeLosslessImage(BufferedImage image) throws IOException { throw unsupported(); }
    public static byte[] encodeImage(BufferedImage image, float quality) throws IOException { throw unsupported(); }
    public static BufferedImage decodeImage(byte[] image) throws IOException { throw unsupported(); }
    public static byte[] createAnimatedWebP(List<BufferedImage> images, int[] delays, Options options) throws IOException { throw unsupported(); }
}
