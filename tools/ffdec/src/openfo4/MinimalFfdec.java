// Copyright 2026 Julian Thomas and OpenFO4 contributors
// SPDX-License-Identifier: Apache-2.0
package openfo4;

import com.jpexs.decompiler.flash.*;
import com.jpexs.decompiler.flash.configuration.Configuration;
import com.jpexs.decompiler.flash.exporters.*;
import com.jpexs.decompiler.flash.exporters.modes.*;
import com.jpexs.decompiler.flash.exporters.settings.*;
import com.jpexs.decompiler.flash.tags.*;
import com.jpexs.decompiler.flash.tags.base.CharacterTag;
import com.jpexs.decompiler.flash.tags.base.ShapeTag;
import java.io.*;
import java.nio.file.*;
import java.util.*;

/** The deliberately limited export interface used by OpenFO4. */
public final class MinimalFfdec {
    public static final String VERSION = "26.2.1-openfo4.1";
    private static final AbortRetryIgnoreHandler FAIL = new AbortRetryIgnoreHandler() {
        public int handle(Throwable error) { throw new ExportFailure(error); }
        public AbortRetryIgnoreHandler getNewInstance() { return this; }
    };
    private static final class ExportFailure extends RuntimeException {
        ExportFailure(Throwable cause) { super(cause); }
    }

    public static void main(String[] args) {
        System.setProperty("java.awt.headless", "true");
        try {
            if (Arrays.equals(args, new String[]{"-help"})) {
                System.out.println("JPEXS Free Flash Decompiler v." + VERSION);
                System.out.println("OpenFO4 font, SVG frame, sprite and shape exporter");
                return;
            }
            run(args);
        } catch (Exception | LinkageError failure) {
            failure.printStackTrace(System.err);
            System.exit(1);
        }
    }

    private static void run(String[] args) throws Exception {
        String format = null, ids = null, selection = null;
        boolean transparent = false;
        int sublength = 1, index = 0;
        while (index < args.length && !args[index].equals("-export")) {
            String option = args[index++];
            switch (option) {
                case "-ignorebackground": transparent = true; break;
                case "-format": format = value(args, index++); break;
                case "-selectid": ids = value(args, index++); break;
                case "-select": selection = value(args, index++); break;
                case "-sublength": sublength = positive(value(args, index++)); break;
                default: throw new IllegalArgumentException("Unsupported option: " + option);
            }
        }
        if (index + 4 != args.length || !args[index].equals("-export"))
            throw new IllegalArgumentException("Expected -export TYPE OUTPUT INPUT");
        String type = args[index + 1];
        Path output = Path.of(args[index + 2]);
        Path input = Path.of(args[index + 3]);
        if (!(type.equals("font") && format == null || type.equals("frame") && "frame:svg".equals(format)
                || type.equals("sprite,shape") && "sprite:svg,shape:svg".equals(format)))
            throw new IllegalArgumentException("Unsupported export type/format: " + type + "/" + format);
        if (!type.equals("sprite,shape") && (ids != null || selection != null))
            throw new IllegalArgumentException("Symbol selection requires sprite,shape export");
        if (type.equals("sprite,shape") && (ids == null || selection == null))
            throw new IllegalArgumentException("Symbol export requires -selectid and -select");
        Configuration.parallelSpeedUp.set(false);
        Files.createDirectories(output);
        try (InputStream stream = Files.newInputStream(input)) {
            SWF swf = new SWF(stream, input.toString(), input.getFileName().toString(), false);
            int aa = Configuration.useMsaaForExport.get() ? Configuration.msaaGridForExport.get() : 1;
            if (type.equals("font")) {
                new FontExporter().exportFonts(FAIL, output.toString(), swf.getTags(),
                        new FontExportSettings(FontExportMode.TTF), swf.getExportEventListener());
            } else if (type.equals("frame")) {
                new FrameExporter().exportFrames(FAIL, output.toString(), swf, 0, frames(swf.frameCount), sublength,
                        new FrameExportSettings(FrameExportMode.SVG, 1, transparent, aa), swf.getExportEventListener());
            } else {
                Set<Integer> selected = new LinkedHashSet<>();
                for (String id : ids.split(",", -1)) selected.add(positive(id));
                Map<Integer, List<Integer>> requested = selections(selection);
                if (!selected.equals(requested.keySet())) throw new IllegalArgumentException("Selection IDs disagree");
                List<Tag> tags = new ArrayList<>();
                for (Tag tag : swf.getTags()) {
                    if (!(tag instanceof CharacterTag character) || !selected.contains(character.getCharacterId())) continue;
                    int id = character.getCharacterId();
                    if (tag instanceof DefineSpriteTag sprite) {
                        List<Integer> wanted = requested.get(id);
                        if (wanted.stream().anyMatch(frame -> frame >= sprite.getFrameCount()))
                            throw new IllegalArgumentException("Sprite frame outside timeline: " + id);
                        new FrameExporter().exportSpriteFrames(FAIL, output.resolve("sprites").toString(), swf, id,
                                wanted, sublength, new SpriteExportSettings(SpriteExportMode.SVG, 1, aa), swf.getExportEventListener());
                    } else if (tag instanceof ShapeTag) {
                        if (!requested.get(id).equals(List.of(0)))
                            throw new IllegalArgumentException("A shape has only frame 1: " + id);
                        tags.add(tag);
                    } else {
                        throw new IllegalArgumentException("Selected symbol is not a sprite or shape: " + id);
                    }
                    selected.remove(id);
                }
                if (!selected.isEmpty()) throw new IllegalArgumentException("Missing selected symbols: " + selected);
                new ShapeExporter().exportShapes(FAIL, output.resolve("shapes").toString(), swf,
                        new ReadOnlyTagList(tags), new ShapeExportSettings(ShapeExportMode.SVG, 1), swf.getExportEventListener(), 1, aa);
            }
        }
        ExportDiagnostics.requireSuccess();
        System.out.println("Export complete: " + input.getFileName());
    }

    private static String value(String[] args, int index) {
        if (index >= args.length) throw new IllegalArgumentException("Missing option value");
        return args[index];
    }
    private static int positive(String value) {
        int number = Integer.parseInt(value);
        if (number <= 0 || number > 65535) throw new IllegalArgumentException("Out-of-range positive integer: " + value);
        return number;
    }
    private static List<Integer> frames(int count) {
        List<Integer> frames = new ArrayList<>();
        for (int frame = 0; frame < count; frame++) frames.add(frame);
        return frames;
    }
    private static Map<Integer, List<Integer>> selections(String value) {
        Map<Integer, List<Integer>> result = new LinkedHashMap<>();
        Integer currentId = null;
        for (String item : value.split(",", -1)) {
            int colon = item.indexOf(':');
            if (colon >= 1) currentId = positive(item.substring(0, colon));
            else if (colon == 0 || currentId == null) throw new IllegalArgumentException("Expected ID:FRAME-RANGES: " + item);
            int id = currentId;
            List<Integer> frames = result.computeIfAbsent(id, ignored -> new ArrayList<>());
            String range = colon < 0 ? item : item.substring(colon + 1);
            int dash = range.indexOf('-');
            int first = positive(dash < 0 ? range : range.substring(0, dash));
            int last = dash < 0 ? first : positive(range.substring(dash + 1));
            if (last < first) throw new IllegalArgumentException("Descending frame range");
            for (int frame = first; frame <= last; frame++) {
                if (frames.contains(frame - 1)) throw new IllegalArgumentException("Duplicate frame selection");
                frames.add(frame - 1);
            }
        }
        for (List<Integer> frames : result.values()) Collections.sort(frames);
        return result;
    }
}
