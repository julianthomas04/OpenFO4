// Copyright 2026 Julian Thomas and OpenFO4 contributors
// SPDX-License-Identifier: Apache-2.0
package com.jpexs.video;

/** No native media player is linked into the minimal SVG/font exporter. */
public final class SimpleMediaPlayer {
    public static boolean isAvailable() {
        unsupported();
        return false;
    }
    public SimpleMediaPlayer() { throw unsupported(); }
    public void addFrameListener(FrameListener listener) { throw unsupported(); }
    public void removeFrameListener(FrameListener listener) { throw unsupported(); }
    public void play(String file) { throw unsupported(); }
    public void stop() { throw unsupported(); }
    public long getLength() { throw unsupported(); }
    public float getPosition() { throw unsupported(); }
    public void setPosition(float position) { throw unsupported(); }
    public boolean isPaused() { throw unsupported(); }
    public void setPaused(boolean paused) { throw unsupported(); }
    public void pause() { throw unsupported(); }
    public boolean isFinished() { throw unsupported(); }
    private static UnsupportedOperationException unsupported() {
        UnsupportedOperationException failure = new UnsupportedOperationException("Embedded SWF video is unsupported");
        openfo4.ExportDiagnostics.unsupported(failure);
        return failure;
    }
}
