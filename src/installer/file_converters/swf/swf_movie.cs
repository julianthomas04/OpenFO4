namespace AssetConverter.Swf
{
    internal readonly record struct SwfColor(byte R, byte G, byte B, byte A);

    /// <summary>
    /// A SWF colour transform: per channel <c>out = in * mult + add</c>, with the multipliers as fractions (the file's terms over 256)
    /// and the add terms in 0..255 units (fractional once composed), channels in R, G, B, A order.
    /// </summary>
    internal sealed record ColorTransform(double RMult, double GMult, double BMult, double AMult, double RAdd, double GAdd, double BAdd, double AAdd)
    {
        public const double TermUnit = 256.0;

        public static readonly ColorTransform Identity = new(1, 1, 1, 1, 0, 0, 0, 0);

        public static ColorTransform FromTerms(IReadOnlyList<int> mult, IReadOnlyList<int> add) =>
            new(mult[0] / TermUnit, mult[1] / TermUnit, mult[2] / TermUnit, mult[3] / TermUnit, add[0], add[1], add[2], add[3]);

        public double[] Multipliers => [RMult, GMult, BMult, AMult];

        public double[] AddTerms => [RAdd, GAdd, BAdd, AAdd];

        /// <summary>The transform that applies <paramref name="child"/> first and this one second.</summary>
        public ColorTransform Compose(ColorTransform child) => new(
            RMult * child.RMult, GMult * child.GMult, BMult * child.BMult, AMult * child.AMult,
            (RMult * child.RAdd) + RAdd, (GMult * child.GAdd) + GAdd, (BMult * child.BAdd) + BAdd, (AMult * child.AAdd) + AAdd);

        public bool HasAddTerms => RAdd != 0 || GAdd != 0 || BAdd != 0 || AAdd != 0;
    }

    internal enum SwfBlendMode
    {
        Normal = 1,
        Layer = 2,
        Multiply = 3,
        Screen = 4,
        Lighten = 5,
        Darken = 6,
        Difference = 7,
        Add = 8,
        Subtract = 9,
        Invert = 10,
        Alpha = 11,
        Erase = 12,
        Overlay = 13,
        Hardlight = 14,
    }

    /// <summary>A read-only list that equals another with the same items in the same order, so records holding one compare by content.</summary>
    internal sealed class EquatableList<T>(IEnumerable<T> items) : IReadOnlyList<T>, IEquatable<EquatableList<T>>
    {
        private readonly T[] _items = [.. items];

        public int Count => _items.Length;

        public T this[int index] => _items[index];

        public IEnumerator<T> GetEnumerator() => ((IEnumerable<T>)_items).GetEnumerator();

        System.Collections.IEnumerator System.Collections.IEnumerable.GetEnumerator() => _items.GetEnumerator();

        public bool Equals(EquatableList<T>? other) => other is not null && _items.SequenceEqual(other._items);

        public override bool Equals(object? obj) => Equals(obj as EquatableList<T>);

        public override int GetHashCode()
        {
            var hash = new HashCode();
            foreach (T item in _items)
                hash.Add(item);
            return hash.ToHashCode();
        }
    }

    internal abstract record SwfFilter;

    internal sealed record DropShadowFilter(
        SwfColor Color, double BlurX, double BlurY, double Angle, double Distance, double Strength,
        bool Inner, bool Knockout, bool CompositeSource, int Passes) : SwfFilter;

    internal sealed record BlurFilter(double BlurX, double BlurY, int Passes) : SwfFilter;

    internal sealed record GlowFilter(
        SwfColor Color, double BlurX, double BlurY, double Strength, bool Inner, bool Knockout, bool CompositeSource, int Passes) : SwfFilter;

    internal sealed record BevelFilter(
        SwfColor ShadowColor, SwfColor HighlightColor, double BlurX, double BlurY, double Angle, double Distance, double Strength,
        bool Inner, bool Knockout, bool CompositeSource, bool OnTop, int Passes) : SwfFilter;

    /// <param name="IsBevel">Gradient bevel when set, gradient glow otherwise.</param>
    internal sealed record GradientFilter(
        bool IsBevel, EquatableList<SwfColor> Colors, EquatableList<int> Ratios, double BlurX, double BlurY, double Angle, double Distance,
        double Strength, bool Inner, bool Knockout, bool CompositeSource, bool OnTop, int Passes) : SwfFilter;

    internal sealed record ConvolutionFilter(
        int MatrixX, int MatrixY, float Divisor, float Bias, EquatableList<float> Matrix, SwfColor DefaultColor, bool Clamp, bool PreserveAlpha) : SwfFilter;

    internal sealed record ColorMatrixFilter(EquatableList<float> Matrix) : SwfFilter;

    /// <summary>One PlaceObject tag: only the properties it states are set; the rest stay as the depth had them.</summary>
    internal sealed record SwfPlaceObject(
        int Depth,
        bool Move,
        int? CharacterId = null,
        string? Name = null,
        SwfMatrix? Matrix = null,
        ColorTransform? Color = null,
        int? Ratio = null,
        int? ClipDepth = null,
        IReadOnlyList<SwfFilter>? Filters = null,
        SwfBlendMode? BlendMode = null,
        bool? CacheAsBitmap = null,
        bool? Visible = null,
        string? ClassName = null,
        SwfColor? Background = null);

    internal sealed record SwfRemoveObject(int Depth);

    /// <summary>The display-list changes and labels of one frame, up to and including its ShowFrame.</summary>
    internal sealed record SwfFrame(IReadOnlyList<string> Labels, IReadOnlyList<object> Actions);

    internal sealed record SwfTimelineTags(int DeclaredFrameCount, IReadOnlyList<SwfFrame> Frames);

    internal abstract record SwfCharacter(int Id)
    {
        public abstract string Kind { get; }
    }

    internal enum SwfFillType
    {
        Solid = 0x00,
        LinearGradient = 0x10,
        RadialGradient = 0x12,
        FocalRadialGradient = 0x13,
        RepeatingBitmap = 0x40,
        ClippedBitmap = 0x41,
        NonSmoothedRepeatingBitmap = 0x42,
        NonSmoothedClippedBitmap = 0x43,
    }

    internal sealed record SwfGradient(int Spread, int Interpolation, IReadOnlyList<(int Ratio, SwfColor Color)> Stops, double? FocalPoint);

    internal sealed record SwfFillStyle(SwfFillType Type, SwfColor? Color = null, SwfMatrix? Matrix = null, SwfGradient? Gradient = null, int? BitmapId = null);

    /// <param name="Width">In twips.</param>
    /// <param name="Caps">Start and end cap (0 round, 1 none, 2 square); set for DefineShape4 line styles only, like the other flags.</param>
    internal sealed record SwfLineStyle(
        int Width, SwfColor? Color, SwfFillStyle? Fill = null, (int Start, int End)? Caps = null, int? Join = null, double? MiterLimit = null,
        bool? NoHScale = null, bool? NoVScale = null, bool? PixelHinting = null, bool? NoClose = null);

    internal readonly record struct SwfPoint(int X, int Y);

    /// <param name="Control">The control point of a quadratic curve; <c>null</c> for a straight edge.</param>
    /// <param name="End">The edge's end point, in twips in the shape's space.</param>
    internal sealed record SwfEdge(SwfPoint? Control, SwfPoint End);

    /// <summary>A run of edges drawn with one set of styles. The style numbers index the shape's <see cref="SwfShape.Fills"/> and <see cref="SwfShape.Lines"/> from 1; 0 is none.</summary>
    /// <param name="FillLeft">The fill on the left of the edges' direction.</param>
    /// <param name="FillRight">The fill on the right of the edges' direction.</param>
    internal sealed record SwfShapePath(int FillLeft, int FillRight, int Line, SwfPoint Start, IReadOnlyList<SwfEdge> Edges);

    /// <param name="Bounds">The box of everything the shape draws, strokes included.</param>
    /// <param name="EdgeBounds">The box of the outline without strokes, stated by DefineShape4 only.</param>
    /// <summary>The flags byte of a DefineShape4: whether overlapping outlines fill by the non-zero winding rule (even-odd otherwise) and how strokes scale.</summary>
    internal sealed record SwfShapeFlags(bool FillWindingRule, bool NonScalingStrokes, bool ScalingStrokes);

    /// <param name="Paths">The outline, in drawing order.</param>
    /// <param name="Flags">Stated by DefineShape4 only.</param>
    internal sealed record SwfShape(
        int Id, int Version, TwipRect Bounds, TwipRect? EdgeBounds, IReadOnlyList<SwfFillStyle> Fills, IReadOnlyList<SwfLineStyle> Lines,
        IReadOnlyList<SwfShapePath> Paths, SwfShapeFlags? Flags = null) : SwfCharacter(Id)
    {
        public override string Kind => "shape";
    }

    internal sealed record SwfSprite(int Id, SwfTimelineTags Timeline) : SwfCharacter(Id)
    {
        public override string Kind => "sprite";
    }

    internal enum SwfTextAlign
    {
        Left = 0,
        Right = 1,
        Center = 2,
        Justify = 3,
    }

    internal sealed record SwfTextLayout(SwfTextAlign Align, int LeftMargin, int RightMargin, int Indent, int Leading);

    internal sealed record SwfEditTextFlags(
        bool WordWrap, bool Multiline, bool Password, bool ReadOnly, bool AutoSize, bool NoSelect, bool Border, bool WasStatic, bool Html, bool UseOutlines);

    /// <param name="FontHeight">In twips.</param>
    internal sealed record SwfEditText(
        int Id, TwipRect Bounds, SwfEditTextFlags Flags, int? FontId, string? FontClass, int? FontHeight, SwfColor? TextColor, int? MaxLength,
        SwfTextLayout? Layout, string VariableName, string? InitialText) : SwfCharacter(Id)
    {
        public override string Kind => "text";
    }

    internal sealed record SwfGlyphEntry(int Index, int Advance);

    internal sealed record SwfTextRun(int? FontId, SwfColor? Color, int? XOffset, int? YOffset, int? Height, IReadOnlyList<SwfGlyphEntry> Glyphs);

    internal sealed record SwfStaticText(int Id, TwipRect Bounds, SwfMatrix Matrix, IReadOnlyList<SwfTextRun> Runs) : SwfCharacter(Id)
    {
        public override string Kind => "statictext";
    }

    /// <param name="EmSize">The units of the metrics: 1024 for DefineFont2, 20480 for DefineFont3.</param>
    internal sealed record SwfFont(
        int Id, int Version, string Name, bool Bold, bool Italic, int EmSize, int? Ascent, int? Descent, int? Leading, IReadOnlyList<int> Codes) : SwfCharacter(Id)
    {
        public override string Kind => "font";
    }

    /// <summary>A character the layout carries only by kind (bitmaps, buttons, sounds, ...), with pixel size where the tag states it.</summary>
    internal sealed record SwfOtherCharacter(int Id, string OtherKind, int? Width = null, int? Height = null) : SwfCharacter(Id)
    {
        public override string Kind => OtherKind;
    }

    internal sealed record SwfAbcBlock(string Name, byte[] Data);

    /// <summary>A text character's CSMTextSettings: the renderer (0 normal, 1 advanced anti-aliasing), grid fitting (0 none, 1 pixel, 2 subpixel), thickness and sharpness.</summary>
    internal sealed record SwfTextSettings(int Renderer, int GridFit, float Thickness, float Sharpness);

    internal sealed record SwfImport(string Url, string Name);

    /// <summary>A parsed SWF movie: header, every character by id, the main timeline and the linkage and script tags.</summary>
    internal sealed record SwfMovie(
        int Version,
        TwipRect Stage,
        double FrameRate,
        SwfTimelineTags Root,
        IReadOnlyDictionary<int, SwfCharacter> Characters,
        IReadOnlyList<(int CharacterId, string Name)> SymbolClasses,
        IReadOnlyList<(int CharacterId, string Name)> Exports,
        IReadOnlyDictionary<int, SwfImport> Imports,
        IReadOnlyDictionary<int, TwipRect> ScalingGrids,
        IReadOnlyDictionary<int, string> FontNames,
        IReadOnlyList<SwfAbcBlock> AbcBlocks,
        IReadOnlyDictionary<int, SwfTextSettings> TextSettings);
}
