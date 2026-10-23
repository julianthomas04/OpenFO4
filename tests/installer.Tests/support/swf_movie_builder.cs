using System.IO.Compression;
using System.Text;
using AssetConverter;
using AssetConverter.Swf;

namespace InstallerTests
{
    /// <summary>The fields of a PlaceObject2/3 tag a test sets; unset fields are left out of the tag.</summary>
    internal sealed record SwfPlacement(int Depth)
    {
        public int? Character { get; init; }
        public string? Name { get; init; }
        public SwfMatrix? Matrix { get; init; }
        public ColorTransform? Color { get; init; }
        public int? Ratio { get; init; }
        public int? ClipDepth { get; init; }
        public bool Move { get; init; }
        public bool? Visible { get; init; }
        public bool? CacheAsBitmap { get; init; }
        public int? BlendMode { get; init; }
        public string? ClassName { get; init; }

        /// <summary>Sets the PlaceObject3 flag that marks the character as a bitmap; the tag carries no field for it.</summary>
        public bool IsImage { get; init; }

        /// <summary>The raw filter list: the count byte and the filters.</summary>
        public byte[]? Filters { get; init; }

        public static ColorTransform Alpha(double multiplier) => ColorTransform.Identity with { AMult = multiplier };

        public static SwfMatrix Translate(double txTwips, double tyTwips) => SwfMatrix.Identity with { Tx = txTwips, Ty = tyTwips };

        /// <summary>A filter list holding one blur filter.</summary>
        public static byte[] BlurFilter(double blurX, double blurY, int passes) =>
            new SwfBitWriter().U8(1).U8(1).Fixed(blurX).Fixed(blurY).UB(passes, 5).UB(0, 3).ToArray();
    }

    /// <summary>The control tags of one timeline (the movie's own or a sprite's), written in the order a test calls them.</summary>
    internal sealed class SwfTimelineWriter
    {
        private readonly List<byte> _tags = [];

        public int ShownFrames { get; private set; }

        public SwfTimelineWriter Place(SwfPlacement placement) => Tag(SwfTagCodes.PlaceObject3, PlaceObjectBody(placement, version3: true));

        public SwfTimelineWriter Place2(SwfPlacement placement) => Tag(SwfTagCodes.PlaceObject2, PlaceObjectBody(placement, version3: false));

        public SwfTimelineWriter Remove(int depth) => Tag(SwfTagCodes.RemoveObject2, new SwfBitWriter().U16(depth).ToArray());

        public SwfTimelineWriter Label(string label) => Tag(SwfTagCodes.FrameLabel, new SwfBitWriter().String(label).ToArray());

        public SwfTimelineWriter ShowFrame()
        {
            ShownFrames++;
            return Tag(SwfTagCodes.ShowFrame, []);
        }

        public SwfTimelineWriter Tag(int code, byte[] body)
        {
            _tags.AddRange(SwfMovieBuilder.TagHeader(code, body.Length));
            _tags.AddRange(body);
            return this;
        }

        public byte[] ToArray() => [.. _tags];

        private static byte[] PlaceObjectBody(SwfPlacement p, bool version3)
        {
            int flags = (p.ClipDepth.HasValue ? 0x40 : 0) | (p.Name != null ? 0x20 : 0) | (p.Ratio.HasValue ? 0x10 : 0) | (p.Color != null ? 0x08 : 0)
                | (p.Matrix.HasValue ? 0x04 : 0) | (p.Character.HasValue ? 0x02 : 0) | (p.Move ? 0x01 : 0);
            int flags2 = (p.Visible.HasValue ? 0x20 : 0) | (p.IsImage ? 0x10 : 0) | (p.ClassName != null ? 0x08 : 0) | (p.CacheAsBitmap.HasValue ? 0x04 : 0)
                | (p.BlendMode.HasValue ? 0x02 : 0) | (p.Filters != null ? 0x01 : 0);
            if (!version3 && flags2 != 0)
                throw new ArgumentException("PlaceObject2 carries no visibility, class name, bitmap caching, blend mode or filters.");
            var w = new SwfBitWriter().U8(flags);
            if (version3)
                w.U8(flags2);
            w.U16(p.Depth);
            if (p.ClassName != null)
                w.String(p.ClassName);
            if (p.Character is { } character)
                w.U16(character);
            if (p.Matrix is { } matrix)
                w.Matrix(matrix);
            if (p.Color != null)
                w.ColorTransform(p.Color);
            if (p.Ratio is { } ratio)
                w.U16(ratio);
            if (p.Name != null)
                w.String(p.Name);
            if (p.ClipDepth is { } clipDepth)
                w.U16(clipDepth);
            if (p.Filters != null)
                w.Bytes(p.Filters);
            if (p.BlendMode is { } blend)
                w.U8(blend);
            if (p.CacheAsBitmap is { } cache)
                w.U8(cache ? 1 : 0);
            if (p.Visible is { } visible)
                w.U8(visible ? 1 : 0);
            return w.ToArray();
        }
    }

    /// <summary>Builds SWF movies byte by byte: header, character definitions, linkage, ActionScript blocks and the main timeline.</summary>
    internal sealed class SwfMovieBuilder
    {
        private readonly List<byte> _definitions = [];

        public int Version { get; init; } = 10;
        public TwipRect Stage { get; init; } = new(0, 0, 1280 * 20, 720 * 20);
        public double FrameRate { get; init; } = 30;
        public bool Compressed { get; init; }

        /// <summary>The frame count the header states; the frames the main timeline shows when unset.</summary>
        public int? DeclaredFrameCount { get; init; }

        public SwfTimelineWriter Root { get; } = new();

        /// <summary>A DefineShape3 (or the given version) of one solid fill and no edges; <paramref name="shapeFlags"/> are the DefineShape4 flag bits.</summary>
        public SwfMovieBuilder Shape(int id, TwipRect bounds, SwfColor fill, int version = 3, TwipRect? edgeBounds = null, byte shapeFlags = 0)
        {
            var w = new SwfBitWriter().U16(id).Rect(bounds);
            if (version == 4)
                w.Rect(edgeBounds ?? bounds).U8(shapeFlags);
            w.U8(1).U8(0x00);
            if (version >= 3)
                w.Rgba(fill.R, fill.G, fill.B, fill.A);
            else
                w.Bytes([fill.R, fill.G, fill.B]);
            w.U8(0);
            w.UB(1, 4).UB(0, 4).UB(0, 6).Align();
            int code = version switch { 1 => SwfTagCodes.DefineShape, 2 => SwfTagCodes.DefineShape2, 3 => SwfTagCodes.DefineShape3, _ => SwfTagCodes.DefineShape4 };
            return Tag(code, w.ToArray());
        }

        /// <summary>A DefineShape3 with one solid fill, one stroke style (when <paramref name="stroke"/> is given) and the outline <paramref name="records"/> draw.</summary>
        public SwfMovieBuilder PathShape(int id, TwipRect bounds, SwfColor fill, SwfColor? stroke, int strokeWidth, Action<SwfShapeRecordWriter> records)
        {
            var w = new SwfBitWriter().U16(id).Rect(bounds).U8(1).U8(0x00).Rgba(fill.R, fill.G, fill.B, fill.A);
            if (stroke is { } line)
                w.U8(1).U16(strokeWidth).Rgba(line.R, line.G, line.B, line.A);
            else
                w.U8(0);
            w.UB(2, 4).UB(stroke is null ? 0 : 1, 4);
            var writer = new SwfShapeRecordWriter(w, stroke is null ? 0 : 1);
            records(writer);
            writer.End();
            return Tag(SwfTagCodes.DefineShape3, w.Align().ToArray());
        }

        /// <summary>A shape without any fill style.</summary>
        public SwfMovieBuilder UnfilledShape(int id, TwipRect bounds) =>
            Tag(SwfTagCodes.DefineShape3, new SwfBitWriter().U16(id).Rect(bounds).U8(0).U8(0).UB(0, 4).UB(0, 4).UB(0, 6).Align().ToArray());

        public SwfMovieBuilder Sprite(int id, Action<SwfTimelineWriter> timeline, int? declaredFrameCount = null)
        {
            var writer = new SwfTimelineWriter();
            timeline(writer);
            byte[] tags = writer.ToArray();
            byte[] body = [.. new SwfBitWriter().U16(id).U16(declaredFrameCount ?? writer.ShownFrames).ToArray(), .. tags, .. TagHeader(SwfTagCodes.End, 0)];
            return Tag(SwfTagCodes.DefineSprite, body);
        }

        public SwfMovieBuilder EditText(
            int id, TwipRect bounds, string? fontClass = null, int? fontHeight = null, string? text = null, bool html = false,
            SwfColor? color = null, string variableName = "", int? fontId = null)
        {
            var w = new SwfBitWriter().U16(id).Rect(bounds);
            w.Flag(text != null).Flag(false).Flag(false).Flag(false).Flag(false).Flag(color.HasValue).Flag(false).Flag(fontId.HasValue)
                .Flag(fontClass != null).Flag(false).Flag(false).Flag(false).Flag(false).Flag(false).Flag(html).Flag(false);
            if (fontId is { } font)
                w.U16(font);
            if (fontClass != null)
                w.String(fontClass);
            if (fontId.HasValue || fontClass != null)
                w.U16(fontHeight ?? 240);
            if (color is { } c)
                w.Rgba(c.R, c.G, c.B, c.A);
            w.String(variableName);
            if (text != null)
                w.String(text);
            return Tag(SwfTagCodes.DefineEditText, w.ToArray());
        }

        /// <summary>
        /// A DefineFont3 without glyphs, as font libraries carry only names for imported fonts; with <paramref name="layout"/> it states
        /// the font's ascent, descent and leading (font units) and no kerning.
        /// </summary>
        public SwfMovieBuilder Font(int id, string name, bool bold = false, bool italic = false, (int Ascent, int Descent, int Leading)? layout = null)
        {
            byte[] nameBytes = Encoding.UTF8.GetBytes(name);
            var w = new SwfBitWriter().U16(id).U8((layout is null ? 0 : 0x80) | (bold ? 0x01 : 0) | (italic ? 0x02 : 0)).U8(0)
                .U8(nameBytes.Length).Bytes(nameBytes).U16(0);
            if (layout is { } metrics)
                w.U16(metrics.Ascent).U16(metrics.Descent).U16(metrics.Leading).U16(0);
            return Tag(SwfTagCodes.DefineFont3, w.ToArray());
        }

        public SwfMovieBuilder SymbolClass(params (int CharacterId, string Name)[] symbols) => Tag(SwfTagCodes.SymbolClass, IdNamePairs(symbols));

        public SwfMovieBuilder Export(params (int CharacterId, string Name)[] symbols) => Tag(SwfTagCodes.ExportAssets, IdNamePairs(symbols));

        public SwfMovieBuilder Import(string url, params (int CharacterId, string Name)[] symbols) =>
            Tag(SwfTagCodes.ImportAssets2, [.. new SwfBitWriter().String(url).U8(1).U8(0).ToArray(), .. IdNamePairs(symbols)]);

        public SwfMovieBuilder ScalingGrid(int characterId, TwipRect grid) => Tag(SwfTagCodes.DefineScalingGrid, new SwfBitWriter().U16(characterId).Rect(grid).ToArray());

        public SwfMovieBuilder Abc(string name, byte[] abc) => Tag(SwfTagCodes.DoAbc, [.. new SwfBitWriter().U32(1).String(name).ToArray(), .. abc]);

        public SwfMovieBuilder Tag(int code, byte[] body)
        {
            _definitions.AddRange(TagHeader(code, body.Length));
            _definitions.AddRange(body);
            return this;
        }

        public byte[] Build()
        {
            byte[] body =
            [
                .. new SwfBitWriter().Rect(Stage).U16((int)Math.Round(FrameRate * 256)).U16(DeclaredFrameCount ?? Root.ShownFrames).ToArray(),
                .. _definitions,
                .. Root.ToArray(),
                .. TagHeader(SwfTagCodes.End, 0),
            ];
            byte[] payload = Compressed ? Deflate(body) : body;
            return [.. Encoding.ASCII.GetBytes(Compressed ? "CWS" : "FWS"), (byte)Version, .. BitConverter.GetBytes(8 + body.Length), .. payload];
        }

        public SwfMovie Read() => SwfMovieReader.Read(Build());

        internal static byte[] TagHeader(int code, int length) =>
            length < 0x3F
                ? BitConverter.GetBytes((ushort)((code << 6) | length))
                : [.. BitConverter.GetBytes((ushort)((code << 6) | 0x3F)), .. BitConverter.GetBytes(length)];

        private static byte[] IdNamePairs((int CharacterId, string Name)[] pairs)
        {
            var w = new SwfBitWriter().U16(pairs.Length);
            foreach ((int id, string name) in pairs)
                w.U16(id).String(name);
            return w.ToArray();
        }

        private static byte[] Deflate(byte[] data)
        {
            using var output = new MemoryStream();
            using (var stream = new ZLibStream(output, CompressionLevel.Optimal, leaveOpen: true))
                stream.Write(data);
            return output.ToArray();
        }
    }
}

namespace InstallerTests
{
    /// <summary>Writes the shape records of a DefineShape3 whose style arrays hold at most three fills and one line style per array.</summary>
    internal sealed class SwfShapeRecordWriter(SwfBitWriter bits, int lineBits)
    {
        private const int EdgeBits = 15;
        private const int FillBits = 2;

        /// <summary>A style change that moves the pen to <paramref name="x"/>, <paramref name="y"/> and selects the given styles (null leaves a style as it is).</summary>
        public SwfShapeRecordWriter Style(int x, int y, int? fillLeft = null, int? fillRight = null, int? line = null, SwfColor[]? newFills = null)
        {
            bits.Flag(false).Flag(newFills is not null).Flag(line is not null).Flag(fillRight is not null).Flag(fillLeft is not null).Flag(true);
            bits.UB(EdgeBits, 5).UB(x, EdgeBits).UB(y, EdgeBits);
            if (fillLeft is { } left)
                bits.UB(left, FillBits);
            if (fillRight is { } right)
                bits.UB(right, FillBits);
            if (line is { } stroke)
                bits.UB(stroke, lineBits);
            if (newFills is not null)
            {
                bits.U8(newFills.Length);
                foreach (SwfColor fill in newFills)
                    bits.U8(0x00).Rgba(fill.R, fill.G, fill.B, fill.A);
                bits.U8(0);

                bits.UB(FillBits, 4).UB(lineBits, 4);
            }
            return this;
        }

        public SwfShapeRecordWriter Line(int dx, int dy)
        {
            bits.Flag(true).Flag(true).UB(EdgeBits - 2, 4).Flag(true).UB(dx, EdgeBits).UB(dy, EdgeBits);
            return this;
        }

        public SwfShapeRecordWriter Curve(int controlDx, int controlDy, int anchorDx, int anchorDy)
        {
            bits.Flag(true).Flag(false).UB(EdgeBits - 2, 4).UB(controlDx, EdgeBits).UB(controlDy, EdgeBits).UB(anchorDx, EdgeBits).UB(anchorDy, EdgeBits);
            return this;
        }

        public void End() => bits.UB(0, 6);
    }
}
