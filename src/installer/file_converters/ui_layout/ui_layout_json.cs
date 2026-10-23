using System.Globalization;
using System.Text.Json;
using AssetConverter.Swf;

namespace AssetConverter.UiLayout
{
    /// <summary>
    /// The JSON shapes of the SWF values in a layout document: lengths in pixels (twips / 20), colours as [r, g, b, a] in 0..255,
    /// colour-transform multipliers as fractions, matrices as [a, b, c, d, tx, ty] (x' = a·x + c·y + tx, y' = b·x + d·y + ty).
    /// Values composed from several placements are rounded to <see cref="ComposedDecimals"/> decimals; read values are written exactly.
    /// </summary>
    internal static class UiLayoutJson
    {
        public const int ComposedDecimals = 6;

        public static double Pixels(double twips) => Twips.ToPixels(twips);

        public static void Matrix(Utf8JsonWriter json, string name, SwfMatrix matrix, bool composed = false)
        {
            json.WriteStartArray(name);
            foreach (double value in new[] { matrix.A, matrix.B, matrix.C, matrix.D, Pixels(matrix.Tx), Pixels(matrix.Ty) })
                json.WriteNumberValue(composed ? Math.Round(value, ComposedDecimals) : value);
            json.WriteEndArray();
        }

        public static void Rect(Utf8JsonWriter json, string name, TwipRect rect)
        {
            json.WriteStartArray(name);
            json.WriteNumberValue(Pixels(rect.Xmin));
            json.WriteNumberValue(Pixels(rect.Ymin));
            json.WriteNumberValue(Pixels(rect.Xmax));
            json.WriteNumberValue(Pixels(rect.Ymax));
            json.WriteEndArray();
        }

        public static void Color(Utf8JsonWriter json, string name, SwfColor color)
        {
            json.WriteStartArray(name);
            json.WriteNumberValue(color.R);
            json.WriteNumberValue(color.G);
            json.WriteNumberValue(color.B);
            json.WriteNumberValue(color.A);
            json.WriteEndArray();
        }

        public static void ColorTransform(Utf8JsonWriter json, string name, Swf.ColorTransform color, bool composed = false)
        {
            json.WriteStartObject(name);
            Numbers(json, "mult", color.Multipliers, composed);
            Numbers(json, "add", color.AddTerms, composed);
            json.WriteEndObject();
        }

        public static void Numbers(Utf8JsonWriter json, string name, IEnumerable<double> values, bool composed = false)
        {
            json.WriteStartArray(name);
            foreach (double value in values)
                json.WriteNumberValue(composed ? Math.Round(value, ComposedDecimals) : value);
            json.WriteEndArray();
        }

        /// <summary>A constant value of the ActionScript: number, string or boolean.</summary>
        public static void Value(Utf8JsonWriter json, object value)
        {
            switch (value)
            {
                case int i:
                    json.WriteNumberValue(i);
                    break;
                case uint u:
                    json.WriteNumberValue(u);
                    break;
                case double d:
                    json.WriteNumberValue(d);
                    break;
                case string s:
                    json.WriteStringValue(s);
                    break;
                case bool b:
                    json.WriteBooleanValue(b);
                    break;
                default:
                    throw new ArgumentException($"{value.GetType().Name} is not a constant value.", nameof(value));
            }
        }

        public static void Placement(Utf8JsonWriter json, SwfDisplayObject placed)
        {
            Matrix(json, "matrix", placed.Matrix);
            ColorTransform(json, "color", placed.Color);
            json.WriteBoolean("visible", placed.Visible);
            if (placed.BlendMode != SwfBlendMode.Normal)
                json.WriteString("blend", placed.BlendMode.ToString().ToLowerInvariant());
            if (placed.ClipDepth is { } clipDepth)
                json.WriteNumber("clipDepth", clipDepth);
            if (placed.Ratio is { } ratio)
                json.WriteNumber("ratio", ratio);
            if (placed.CacheAsBitmap)
                json.WriteBoolean("cacheAsBitmap", true);
            if (placed.ClassName is { } className)
                json.WriteString("className", className);
            if (placed.Background is { } background)
                Color(json, "background", background);
            if (placed.Filters.Count > 0)
            {
                json.WriteStartArray("filters");
                foreach (SwfFilter filter in placed.Filters)
                    Filter(json, filter);
                json.WriteEndArray();
            }
        }

        private static void Filter(Utf8JsonWriter json, SwfFilter filter)
        {
            json.WriteStartObject();
            switch (filter)
            {
                case DropShadowFilter f:
                    json.WriteString("type", "dropShadow");
                    Color(json, "color", f.Color);
                    Blur(json, f.BlurX, f.BlurY, f.Passes);
                    json.WriteNumber("angle", f.Angle);
                    json.WriteNumber("distance", f.Distance);
                    json.WriteNumber("strength", f.Strength);
                    Flags(json, f.Inner, f.Knockout, f.CompositeSource, null);
                    break;
                case BlurFilter f:
                    json.WriteString("type", "blur");
                    Blur(json, f.BlurX, f.BlurY, f.Passes);
                    break;
                case GlowFilter f:
                    json.WriteString("type", "glow");
                    Color(json, "color", f.Color);
                    Blur(json, f.BlurX, f.BlurY, f.Passes);
                    json.WriteNumber("strength", f.Strength);
                    Flags(json, f.Inner, f.Knockout, f.CompositeSource, null);
                    break;
                case BevelFilter f:
                    json.WriteString("type", "bevel");
                    Color(json, "shadowColor", f.ShadowColor);
                    Color(json, "highlightColor", f.HighlightColor);
                    Blur(json, f.BlurX, f.BlurY, f.Passes);
                    json.WriteNumber("angle", f.Angle);
                    json.WriteNumber("distance", f.Distance);
                    json.WriteNumber("strength", f.Strength);
                    Flags(json, f.Inner, f.Knockout, f.CompositeSource, f.OnTop);
                    break;
                case GradientFilter f:
                    json.WriteString("type", f.IsBevel ? "gradientBevel" : "gradientGlow");
                    json.WriteStartArray("stops");
                    for (int i = 0; i < f.Colors.Count; i++)
                    {
                        json.WriteStartObject();
                        json.WriteNumber("ratio", f.Ratios[i]);
                        Color(json, "color", f.Colors[i]);
                        json.WriteEndObject();
                    }
                    json.WriteEndArray();
                    Blur(json, f.BlurX, f.BlurY, f.Passes);
                    json.WriteNumber("angle", f.Angle);
                    json.WriteNumber("distance", f.Distance);
                    json.WriteNumber("strength", f.Strength);
                    Flags(json, f.Inner, f.Knockout, f.CompositeSource, f.OnTop);
                    break;
                case ConvolutionFilter f:
                    json.WriteString("type", "convolution");
                    json.WriteNumber("matrixX", f.MatrixX);
                    json.WriteNumber("matrixY", f.MatrixY);
                    json.WriteNumber("divisor", f.Divisor);
                    json.WriteNumber("bias", f.Bias);
                    Numbers(json, "matrix", f.Matrix.Select(v => (double)v));
                    Color(json, "defaultColor", f.DefaultColor);
                    json.WriteBoolean("clamp", f.Clamp);
                    json.WriteBoolean("preserveAlpha", f.PreserveAlpha);
                    break;
                case ColorMatrixFilter f:
                    json.WriteString("type", "colorMatrix");
                    Numbers(json, "matrix", f.Matrix.Select(v => (double)v));
                    break;
                default:
                    throw new ArgumentException($"{filter.GetType().Name} is not a SWF filter.", nameof(filter));
            }
            json.WriteEndObject();
        }

        private static void Blur(Utf8JsonWriter json, double blurX, double blurY, int passes)
        {
            json.WriteNumber("blurX", blurX);
            json.WriteNumber("blurY", blurY);
            json.WriteNumber("passes", passes);
        }

        private static void Flags(Utf8JsonWriter json, bool inner, bool knockout, bool compositeSource, bool? onTop)
        {
            json.WriteBoolean("inner", inner);
            json.WriteBoolean("knockout", knockout);
            json.WriteBoolean("compositeSource", compositeSource);
            if (onTop is { } top)
                json.WriteBoolean("onTop", top);
        }

        public static string Invariant(int value) => value.ToString(CultureInfo.InvariantCulture);
    }
}
