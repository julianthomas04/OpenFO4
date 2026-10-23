// Copyright 2026 Julian Thomas and OpenFO4 contributors
// SPDX-License-Identifier: Apache-2.0
package openfo4;

import java.io.IOException;
import java.util.concurrent.atomic.AtomicReference;

/** Preserves required failures even where upstream rendering code catches an exception. */
public final class ExportDiagnostics {
    private static final AtomicReference<Throwable> FAILURE = new AtomicReference<>();
    public static void unsupported(Throwable cause) { FAILURE.compareAndSet(null, cause); }
    public static void requireSuccess() throws IOException {
        Throwable cause = FAILURE.get();
        if (cause != null) throw new IOException("The exporter encountered an unsupported image or codec; no partial export is valid", cause);
    }
}
