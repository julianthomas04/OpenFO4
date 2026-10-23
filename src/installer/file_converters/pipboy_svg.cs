using System.Globalization;
using System.Text.RegularExpressions;

namespace AssetConverter
{
    public sealed class SvgFormatException(string message) : Exception(message) { }

    /// <summary>A symbol-local rectangle in stage pixels; the registration point is the origin.</summary>
    public readonly record struct ArtBounds(double XMin, double YMin, double XMax, double YMax)
    {
        public double Width => XMax - XMin;

        public double Height => YMax - YMin;

        public ArtBounds Union(ArtBounds other) =>
            new(Math.Min(XMin, other.XMin), Math.Min(YMin, other.YMin), Math.Max(XMax, other.XMax), Math.Max(YMax, other.YMax));

        public double[] ToArray() => [XMin, YMin, XMax, YMax];
    }

    /// <summary>
    /// One exported frame: the exporter lays a symbol out on a canvas of its own size and moves the registration point
    /// to an offset with a group transform. <see cref="Bounds"/> undoes that offset, so the frame can be re-emitted
    /// with a shared viewBox in registration-point coordinates.
    /// </summary>
    public sealed partial class SvgFrame
    {
        const string IdentityOriginGroup = "<g transform=\"matrix(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)\">";

        [GeneratedRegex(@"<svg\b[^>]*>")]
        private static partial Regex RootElement();

        [GeneratedRegex(@"(?<=\s)(?<name>width|height)=""(?<value>[^""]*)""")]
        private static partial Regex SizeAttribute();

        [GeneratedRegex(@"<g transform=""matrix\(1\.0, 0\.0, 0\.0, 1\.0, (-?[0-9.]+), (-?[0-9.]+)\)"">")]
        private static partial Regex OriginGroup();

        [GeneratedRegex(@"\A\s*<g transform=""matrix\([^""]*\)""\s*/>")]
        private static partial Regex EmptyOriginGroup();

        readonly string _svg;

        SvgFrame(string svg, ArtBounds? bounds)
        {
            _svg = svg;
            Bounds = bounds;
        }

        /// <summary>Null for a frame without content: no canvas, or a canvas whose origin group is empty (a blank frame).</summary>
        public ArtBounds? Bounds { get; }

        public static SvgFrame Parse(string svg)
        {
            var root = RootElement().Match(svg);
            if (!root.Success) throw new SvgFormatException("Exported frame has no <svg> element.");

            double width = Size(root.Value, "width"), height = Size(root.Value, "height");
            if (width <= 0 || height <= 0) return new SvgFrame(svg, null);

            if (EmptyOriginGroup().IsMatch(svg.AsSpan(root.Index + root.Length))) return new SvgFrame(svg, null);

            var origin = OriginGroup().Match(svg);
            if (!origin.Success) throw new SvgFormatException("Exported frame has no origin group.");
            double tx = double.Parse(origin.Groups[1].Value, CultureInfo.InvariantCulture);
            double ty = double.Parse(origin.Groups[2].Value, CultureInfo.InvariantCulture);
            return new SvgFrame(svg, new ArtBounds(Round(-tx), Round(-ty), Round(width - tx), Round(height - ty)));
        }

        /// <summary>The canvas size the root element states, in pixels with or without the unit.</summary>
        static double Size(string root, string name)
        {
            var attribute = SizeAttribute().Matches(root).FirstOrDefault(match => match.Groups["name"].Value == name)
                ?? throw new SvgFormatException($"Exported frame states no {name}.");
            string text = attribute.Groups["value"].Value.Trim();
            if (text.EndsWith("px", StringComparison.Ordinal)) text = text[..^2];
            return double.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out double value) && double.IsFinite(value)
                ? value
                : throw new SvgFormatException($"Exported frame states the unreadable {name} '{attribute.Groups["value"].Value}'.");
        }

        static double Round(double value) => Math.Round(value, 4);

        /// <summary>The frame re-emitted with <paramref name="bounds"/> as its viewBox and the registration point at the origin.</summary>
        public string Rebase(ArtBounds bounds)
        {
            double width = Round(bounds.Width), height = Round(bounds.Height);
            string viewBox = string.Create(CultureInfo.InvariantCulture, $"{bounds.XMin} {bounds.YMin} {width} {height}");
            string size = string.Create(CultureInfo.InvariantCulture, $"width=\"{width}\" height=\"{height}\" viewBox=\"{viewBox}\"");
            string header = $"<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" xmlns:ffdec=\"https://www.free-decompiler.com/flash\" {size}>";
            if (Bounds is null) return header + "</svg>";

            var root = RootElement().Match(_svg);
            int content = root.Index + root.Length;
            var origin = OriginGroup().Match(_svg, content);
            if (!origin.Success) return string.Concat(header, _svg.AsSpan(content));
            return string.Concat(header, _svg.AsSpan(content, origin.Index - content), IdentityOriginGroup, _svg.AsSpan(origin.Index + origin.Length));
        }
    }
}
