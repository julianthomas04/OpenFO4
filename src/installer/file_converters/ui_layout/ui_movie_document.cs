using System.Globalization;
using System.Text;
using System.Text.Json;
using AssetConverter.Abc;
using AssetConverter.Swf;

namespace AssetConverter.UiLayout
{
    /// <summary>
    /// Writes the layout document of one movie: header and source fingerprint, stage, timelines, every character, the instance paths,
    /// and the ActionScript constants (<c>constants</c>, <c>literals</c>, <c>unresolved</c>). The engine's
    /// loader mirrors the schema.
    /// </summary>
    internal static class UiMovieDocument
    {
        public static byte[] Write(string movieName, ConversionStamp stamp, SwfMovie movie, AbcConstantMap scripts)
        {
            var exported = ExportedSymbols(movie);
            UiInstanceTree tree = UiInstanceTreeBuilder.Build(movie, exported.Where(e => e.CharacterId != 0));
            var classByCharacter = exported.GroupBy(e => e.CharacterId).ToDictionary(g => g.Key, g => g.First().Class);

            using var buffer = new MemoryStream();
            using (var json = new Utf8JsonWriter(buffer, UiDocumentHeader.WriterOptions))
            {
                json.WriteStartObject();
                UiDocumentHeader.Write(json, UiDocumentHeader.MovieKind, stamp);
                json.WriteString("movie", movieName);
                json.WriteNumber("swfVersion", movie.Version);
                UiLayoutJson.Rect(json, "stage", movie.Stage);
                json.WriteNumber("fps", movie.FrameRate);
                if (classByCharacter.TryGetValue(0, out string? documentClass))
                    json.WriteString("documentClass", documentClass);
                WriteTimelineSummary(json, SwfTimelinePlayer.Play(movie.Root, "the main timeline"));
                WriteSymbols(json, exported, movie.Imports);
                WriteCharacters(json, movie, classByCharacter);
                WriteInstances(json, tree);
                WriteScripts(json, scripts, tree.Unresolved);
                json.WriteEndObject();
            }
            return buffer.ToArray();
        }

        private static List<(string Class, int CharacterId)> ExportedSymbols(SwfMovie movie) =>
            movie.SymbolClasses.Concat(movie.Exports).Select(p => (p.Name, p.CharacterId)).Distinct().ToList();

        private static void WriteTimelineSummary(Utf8JsonWriter json, SwfPlayedTimeline timeline)
        {
            json.WriteNumber("frameCount", timeline.FrameCount);
            WriteLabels(json, timeline.Labels);
        }

        private static void WriteLabels(Utf8JsonWriter json, IReadOnlyDictionary<string, int> labels)
        {
            json.WriteStartObject("labels");
            foreach ((string label, int frame) in labels.OrderBy(l => l.Value).ThenBy(l => l.Key, StringComparer.Ordinal))
                json.WriteNumber(label, frame);
            json.WriteEndObject();
        }

        private static void WriteSymbols(Utf8JsonWriter json, List<(string Class, int CharacterId)> exported, IReadOnlyDictionary<int, SwfImport> imports)
        {
            json.WriteStartObject("symbols");
            foreach (var group in exported.GroupBy(e => e.Class, StringComparer.Ordinal).OrderBy(g => g.Key, StringComparer.Ordinal))
            {
                int[] characters = [.. group.Select(e => e.CharacterId).Distinct()];
                json.WriteNumber(group.Key, characters.Length == 1
                    ? characters[0]
                    : throw new SwfFormatException($"The class '{group.Key}' is exported for {characters.Length} different characters."));
            }
            json.WriteEndObject();
            json.WriteStartObject("imports");
            foreach ((int id, SwfImport import) in imports.OrderBy(i => i.Key))
            {
                json.WriteStartObject(UiLayoutJson.Invariant(id));
                json.WriteString("url", import.Url);
                json.WriteString("name", import.Name);
                json.WriteEndObject();
            }
            json.WriteEndObject();
        }

        private static void WriteCharacters(Utf8JsonWriter json, SwfMovie movie, Dictionary<int, string> classByCharacter)
        {
            json.WriteStartObject("characters");
            foreach (SwfCharacter character in movie.Characters.Values.OrderBy(c => c.Id))
            {
                json.WriteStartObject(UiLayoutJson.Invariant(character.Id));
                json.WriteString("kind", character.Kind);
                if (classByCharacter.TryGetValue(character.Id, out string? className))
                    json.WriteString("class", className);
                if (movie.ScalingGrids.TryGetValue(character.Id, out TwipRect grid))
                    UiLayoutJson.Rect(json, "scalingGrid", grid);
                switch (character)
                {
                    case SwfShape shape:
                        WriteShape(json, shape);
                        break;
                    case SwfSprite sprite:
                        WriteTimelineSummary(json, SwfTimelinePlayer.Play(sprite.Timeline, $"sprite {sprite.Id}"));
                        break;
                    case SwfEditText text:
                        WriteEditText(json, text, movie);
                        break;
                    case SwfStaticText text:
                        WriteStaticText(json, text, movie);
                        break;
                    case SwfFont font:
                        WriteFont(json, font, movie);
                        break;
                    case SwfOtherCharacter other:
                        if (other.Width is { } width && other.Height is { } height)
                        {
                            json.WriteNumber("width", width);
                            json.WriteNumber("height", height);
                        }
                        break;
                }
                json.WriteEndObject();
            }
            json.WriteEndObject();
        }

        private static void WriteShape(Utf8JsonWriter json, SwfShape shape)
        {
            UiLayoutJson.Rect(json, "bounds", shape.Bounds);
            if (shape.EdgeBounds is { } edges)
                UiLayoutJson.Rect(json, "edgeBounds", edges);
            json.WriteStartArray("fills");
            foreach (SwfFillStyle fill in shape.Fills)
                WriteFill(json, fill);
            json.WriteEndArray();
            json.WriteStartArray("lines");
            foreach (SwfLineStyle line in shape.Lines)
            {
                json.WriteStartObject();
                json.WriteNumber("width", UiLayoutJson.Pixels(line.Width));
                if (line.Color is { } color)
                    UiLayoutJson.Color(json, "color", color);
                if (line.Fill is { } fill)
                {
                    json.WritePropertyName("fill");
                    WriteFill(json, fill);
                }
                if (line.Caps is { } caps)
                {
                    json.WriteString("startCap", CapName(caps.Start));
                    json.WriteString("endCap", CapName(caps.End));
                }
                if (line.Join is { } join)
                    json.WriteString("join", JoinName(join));
                if (line.MiterLimit is { } miter)
                    json.WriteNumber("miterLimit", miter);
                if (line.NoHScale is { } noH)
                    json.WriteBoolean("noHScale", noH);
                if (line.NoVScale is { } noV)
                    json.WriteBoolean("noVScale", noV);
                if (line.PixelHinting is { } hinting)
                    json.WriteBoolean("pixelHinting", hinting);
                if (line.NoClose is { } noClose)
                    json.WriteBoolean("noClose", noClose);
                json.WriteEndObject();
            }
            json.WriteEndArray();
            if (shape.Flags is { } flags)
            {
                json.WriteBoolean("fillWindingRule", flags.FillWindingRule);
                json.WriteBoolean("nonScalingStrokes", flags.NonScalingStrokes);
                json.WriteBoolean("scalingStrokes", flags.ScalingStrokes);
            }
            json.WriteStartArray("paths");
            foreach (SwfShapePath path in shape.Paths)
                WritePath(json, path);
            json.WriteEndArray();
        }

        /// <summary>
        /// One run of edges: the style numbers (1-based into <c>fills</c> and <c>lines</c>, 0 for none) on its left and right and of its stroke,
        /// its start point and each edge's end point <c>[x, y]</c> or quadratic curve <c>[controlX, controlY, x, y]</c>, in pixels.
        /// </summary>
        private static void WritePath(Utf8JsonWriter json, SwfShapePath path)
        {
            json.WriteStartObject();
            json.WriteNumber("fillLeft", path.FillLeft);
            json.WriteNumber("fillRight", path.FillRight);
            json.WriteNumber("line", path.Line);
            Point(json, "start", path.Start);
            json.WriteStartArray("edges");
            foreach (SwfEdge edge in path.Edges)
            {
                json.WriteStartArray();
                if (edge.Control is { } control)
                {
                    json.WriteNumberValue(UiLayoutJson.Pixels(control.X));
                    json.WriteNumberValue(UiLayoutJson.Pixels(control.Y));
                }
                json.WriteNumberValue(UiLayoutJson.Pixels(edge.End.X));
                json.WriteNumberValue(UiLayoutJson.Pixels(edge.End.Y));
                json.WriteEndArray();
            }
            json.WriteEndArray();
            json.WriteEndObject();
        }

        private static void Point(Utf8JsonWriter json, string name, SwfPoint point)
        {
            json.WriteStartArray(name);
            json.WriteNumberValue(UiLayoutJson.Pixels(point.X));
            json.WriteNumberValue(UiLayoutJson.Pixels(point.Y));
            json.WriteEndArray();
        }

        private static string CapName(int cap) => cap switch
        {
            0 => "round",
            1 => "none",
            2 => "square",
            _ => throw new SwfFormatException($"A line style names cap style {cap}."),
        };

        private static string JoinName(int join) => join switch
        {
            0 => "round",
            1 => "bevel",
            2 => "miter",
            _ => throw new SwfFormatException($"A line style names join style {join}."),
        };

        private static void WriteFill(Utf8JsonWriter json, SwfFillStyle fill)
        {
            json.WriteStartObject();
            json.WriteString("type", fill.Type switch
            {
                SwfFillType.Solid => "solid",
                SwfFillType.LinearGradient => "linear",
                SwfFillType.RadialGradient => "radial",
                SwfFillType.FocalRadialGradient => "focal",
                _ => "bitmap",
            });
            if (fill.Color is { } color)
                UiLayoutJson.Color(json, "color", color);
            if (fill.Matrix is { } matrix)
                UiLayoutJson.Matrix(json, "matrix", matrix);
            if (fill.Gradient is { } gradient)
            {
                json.WriteNumber("spread", gradient.Spread);
                json.WriteNumber("interpolation", gradient.Interpolation);
                if (gradient.FocalPoint is { } focal)
                    json.WriteNumber("focalPoint", focal);
                json.WriteStartArray("stops");
                foreach ((int ratio, SwfColor stop) in gradient.Stops)
                {
                    json.WriteStartObject();
                    json.WriteNumber("ratio", ratio);
                    UiLayoutJson.Color(json, "color", stop);
                    json.WriteEndObject();
                }
                json.WriteEndArray();
            }
            if (fill.BitmapId is { } bitmap)
            {
                json.WriteNumber("bitmap", bitmap);
                json.WriteBoolean("repeat", fill.Type is SwfFillType.RepeatingBitmap or SwfFillType.NonSmoothedRepeatingBitmap);
                json.WriteBoolean("smooth", fill.Type is SwfFillType.RepeatingBitmap or SwfFillType.ClippedBitmap);
            }
            json.WriteEndObject();
        }

        private static void WriteEditText(Utf8JsonWriter json, SwfEditText text, SwfMovie movie)
        {
            UiLayoutJson.Rect(json, "bounds", text.Bounds);
            if (text.FontId is { } fontId)
                json.WriteNumber("fontId", fontId);
            if (FontName(text, movie) is { } font)
                json.WriteString("font", font);
            if (text.FontHeight is { } height)
                json.WriteNumber("size", UiLayoutJson.Pixels(height));
            if (text.TextColor is { } color)
                UiLayoutJson.Color(json, "color", color);
            if (text.MaxLength is { } maxLength)
                json.WriteNumber("maxLength", maxLength);
            if (text.Layout is { } layout)
            {
                json.WriteString("align", layout.Align.ToString().ToLowerInvariant());
                json.WriteNumber("leftMargin", UiLayoutJson.Pixels(layout.LeftMargin));
                json.WriteNumber("rightMargin", UiLayoutJson.Pixels(layout.RightMargin));
                json.WriteNumber("indent", UiLayoutJson.Pixels(layout.Indent));
                json.WriteNumber("leading", UiLayoutJson.Pixels(layout.Leading));
            }
            SwfEditTextFlags flags = text.Flags;
            json.WriteStartObject("flags");
            json.WriteBoolean("wordWrap", flags.WordWrap);
            json.WriteBoolean("multiline", flags.Multiline);
            json.WriteBoolean("password", flags.Password);
            json.WriteBoolean("readOnly", flags.ReadOnly);
            json.WriteBoolean("autoSize", flags.AutoSize);
            json.WriteBoolean("noSelect", flags.NoSelect);
            json.WriteBoolean("border", flags.Border);
            json.WriteBoolean("wasStatic", flags.WasStatic);
            json.WriteBoolean("html", flags.Html);
            json.WriteBoolean("useOutlines", flags.UseOutlines);
            json.WriteEndObject();
            json.WriteString("variable", text.VariableName);
            if (text.InitialText is { } initial)
            {
                json.WriteString("initialText", initial);
                if (flags.Html)
                    WriteHtmlFormat(json, initial);
            }
            WriteTextSettings(json, text.Id, movie);
        }

        /// <summary>The name a text field's font resolves to: its font class, else the font character's name, DefineFontName or import name.</summary>
        private static string? FontName(SwfEditText text, SwfMovie movie)
        {
            if (text.FontClass is { } fontClass)
                return fontClass;
            if (text.FontId is not { } fontId)
                return null;
            if (movie.Characters.GetValueOrDefault(fontId) is SwfFont { Name.Length: > 0 } font)
                return font.Name;
            if (movie.FontNames.TryGetValue(fontId, out string? named))
                return named;
            return movie.Imports.TryGetValue(fontId, out SwfImport? import) ? import.Name : null;
        }

        /// <summary>
        /// The format of an HTML initial text: <c>htmlFormat</c>, the first value of each <c>TAG.ATTRIBUTE</c> (letter spacing, kerning,
        /// leading live only there), and <c>htmlRuns</c>, the text in runs with the format in force over each.
        /// </summary>
        private static void WriteHtmlFormat(Utf8JsonWriter json, string html)
        {
            HtmlText parsed = HtmlText.Parse(html);
            json.WriteStartObject("htmlFormat");
            foreach ((string key, string value) in parsed.Attributes.OrderBy(a => a.Key, StringComparer.Ordinal))
                WriteFormatValue(json, key, value);
            json.WriteEndObject();
            json.WriteStartArray("htmlRuns");
            foreach (HtmlTextRun run in parsed.Runs)
            {
                json.WriteStartObject();
                json.WriteString("text", run.Text);
                json.WriteStartObject("format");
                foreach ((string key, string value) in run.Format)
                    WriteFormatValue(json, key, value);
                json.WriteEndObject();
                json.WriteEndObject();
            }
            json.WriteEndArray();
        }

        private static void WriteFormatValue(Utf8JsonWriter json, string key, string value)
        {
            if (!key.Contains('.', StringComparison.Ordinal) && value == HtmlText.StyleOn)
                json.WriteBoolean(key, true);
            else if (double.TryParse(value, NumberStyles.Float, CultureInfo.InvariantCulture, out double number))
                json.WriteNumber(key, number);
            else
                json.WriteString(key, value);
        }

        private static void WriteTextSettings(Utf8JsonWriter json, int characterId, SwfMovie movie)
        {
            if (!movie.TextSettings.TryGetValue(characterId, out SwfTextSettings? settings))
                return;
            json.WriteStartObject("textSettings");
            json.WriteString("renderer", settings.Renderer switch
            {
                0 => "normal",
                1 => "advanced",
                _ => throw new SwfFormatException($"Text {characterId} names text renderer {settings.Renderer}."),
            });
            json.WriteString("gridFit", settings.GridFit switch
            {
                0 => "none",
                1 => "pixel",
                2 => "subpixel",
                _ => throw new SwfFormatException($"Text {characterId} names grid fit {settings.GridFit}."),
            });
            json.WriteNumber("thickness", settings.Thickness);
            json.WriteNumber("sharpness", settings.Sharpness);
            json.WriteEndObject();
        }

        private static void WriteStaticText(Utf8JsonWriter json, SwfStaticText text, SwfMovie movie)
        {
            UiLayoutJson.Rect(json, "bounds", text.Bounds);
            UiLayoutJson.Matrix(json, "matrix", text.Matrix);
            WriteTextSettings(json, text.Id, movie);
            json.WriteStartArray("runs");
            int? fontId = null;
            foreach (SwfTextRun run in text.Runs)
            {
                fontId = run.FontId ?? fontId;
                json.WriteStartObject();
                if (run.FontId is { } id)
                    json.WriteNumber("fontId", id);
                if (run.Height is { } height)
                    json.WriteNumber("size", UiLayoutJson.Pixels(height));
                if (run.Color is { } color)
                    UiLayoutJson.Color(json, "color", color);
                if (run.XOffset is { } x)
                    json.WriteNumber("x", UiLayoutJson.Pixels(x));
                if (run.YOffset is { } y)
                    json.WriteNumber("y", UiLayoutJson.Pixels(y));
                json.WriteNumber("glyphs", run.Glyphs.Count);
                WriteRunText(json, run, fontId, movie, text.Id);
                json.WriteEndObject();
            }
            json.WriteEndArray();
        }

        /// <summary>The characters of a run when its font is defined in the movie; otherwise <c>textUnavailable</c> says why they are not known.</summary>
        private static void WriteRunText(Utf8JsonWriter json, SwfTextRun run, int? fontId, SwfMovie movie, int textId)
        {
            if (run.Glyphs.Count == 0)
                return;
            if (fontId is not { } font)
            {
                json.WriteString("textUnavailable", "the text states no font");
                return;
            }
            if (movie.Imports.TryGetValue(font, out SwfImport? import))
            {
                json.WriteString("textUnavailable", $"its font is imported from '{import.Url}'");
                return;
            }
            if (movie.Characters.GetValueOrDefault(font) is not SwfFont { Codes.Count: > 0 } definition)
            {
                json.WriteString("textUnavailable", "its font states no glyph codes");
                return;
            }
            var decoded = new StringBuilder();
            foreach (SwfGlyphEntry glyph in run.Glyphs)
            {
                if (glyph.Index >= definition.Codes.Count)
                    throw new SwfFormatException($"Text {textId} uses glyph {glyph.Index} of font {font}, which has {definition.Codes.Count}.");
                decoded.Append(checked((char)definition.Codes[glyph.Index]));
            }
            json.WriteString("text", decoded.ToString());
        }

        private static void WriteFont(Utf8JsonWriter json, SwfFont font, SwfMovie movie)
        {
            json.WriteString("name", font.Name.Length > 0 ? font.Name : movie.FontNames.GetValueOrDefault(font.Id, ""));
            json.WriteBoolean("bold", font.Bold);
            json.WriteBoolean("italic", font.Italic);
            json.WriteNumber("glyphCount", font.Codes.Count);
            if (font.EmSize > 0)
                json.WriteNumber("emSize", font.EmSize);
            if (font.Ascent is { } ascent && font.Descent is { } descent && font.Leading is { } leading)
            {
                json.WriteNumber("ascent", ascent);
                json.WriteNumber("descent", descent);
                json.WriteNumber("leading", leading);
            }
        }

        private static void WriteInstances(Utf8JsonWriter json, UiInstanceTree tree)
        {
            json.WriteStartObject("instances");
            var written = new HashSet<string>(StringComparer.Ordinal);
            foreach (UiInstance instance in tree.Instances)
            {
                if (!written.Add(instance.Path))
                    throw new SwfFormatException($"The instance path '{instance.Path}' was produced twice.");
                json.WriteStartObject(instance.Path);
                json.WriteNumber("char", instance.CharacterId);
                json.WriteString("kind", instance.Kind);
                json.WriteNumber("depth", instance.Local.Depth);
                if (instance.Local.Name is { } name)
                    json.WriteString("name", name);
                json.WriteStartArray("frames");
                json.WriteNumberValue(instance.FirstFrame);
                json.WriteNumberValue(instance.LastFrame);
                json.WriteEndArray();
                json.WriteStartObject("local");
                UiLayoutJson.Placement(json, instance.Local);
                json.WriteEndObject();
                json.WriteStartObject("world");
                UiLayoutJson.Matrix(json, "matrix", instance.WorldMatrix, composed: true);
                UiLayoutJson.ColorTransform(json, "color", instance.WorldColor, composed: true);
                json.WriteEndObject();
                if (instance.Track.Count > 0)
                {
                    json.WriteStartArray("track");
                    foreach (UiTrackKey key in instance.Track)
                    {
                        json.WriteStartObject();
                        json.WriteNumber("frame", key.Frame);
                        UiLayoutJson.Placement(json, key.State);
                        json.WriteEndObject();
                    }
                    json.WriteEndArray();
                }
                json.WriteEndObject();
            }
            json.WriteEndObject();
        }

        private static void WriteScripts(Utf8JsonWriter json, AbcConstantMap scripts, IReadOnlyDictionary<string, string> instanceUnresolved)
        {
            json.WriteStartObject("constants");
            foreach ((string key, object value) in scripts.Constants)
            {
                json.WritePropertyName(key);
                UiLayoutJson.Value(json, value);
            }
            json.WriteEndObject();
            json.WriteStartArray("constantsFolded");
            foreach (string key in scripts.FoldedConstants)
                json.WriteStringValue(key);
            json.WriteEndArray();
            json.WriteStartObject("literals");
            foreach ((string key, AbcLiteralList list) in scripts.Literals)
            {
                json.WriteStartObject(key);
                json.WriteStartArray("values");
                foreach (object value in list.Values)
                    UiLayoutJson.Value(json, value);
                json.WriteEndArray();
                if (list.Folded.Count > 0)
                {
                    json.WriteStartArray("folded");
                    foreach (int index in list.Folded)
                        json.WriteNumberValue(index);
                    json.WriteEndArray();
                }
                json.WriteEndObject();
            }
            json.WriteEndObject();
            json.WriteStartObject("superClasses");
            foreach ((string className, string superClass) in scripts.SuperClasses.OrderBy(p => p.Key, StringComparer.Ordinal))
                json.WriteString(className, superClass);
            json.WriteEndObject();
            json.WriteStartObject("referenceLists");
            foreach ((string key, IReadOnlyList<string> names) in scripts.ReferenceLists)
            {
                json.WriteStartArray(key);
                foreach (string name in names)
                    json.WriteStringValue(name);
                json.WriteEndArray();
            }
            json.WriteEndObject();
            json.WriteStartObject("unresolved");
            foreach ((string key, string reason) in scripts.Unresolved.Concat(instanceUnresolved).OrderBy(p => p.Key, StringComparer.Ordinal))
                json.WriteString(key, reason);
            json.WriteEndObject();
        }
    }
}
