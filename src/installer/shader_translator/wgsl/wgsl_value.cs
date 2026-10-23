using System.Globalization;

namespace AssetConverter.ShaderTranslation.Wgsl
{
    internal enum Kind { F32, I32, U32, Bool }

    /// <summary>A WGSL expression together with its scalar kind and vector width (1-4).</summary>
    internal readonly record struct Val(string Text, Kind Kind, int Width)
    {
        public string Type => WgslText.Type(Kind, Width);
    }

    internal static class WgslText
    {
        /// <summary>Appends a line terminated by LF regardless of platform.</summary>
        public static System.Text.StringBuilder AppendLn(this System.Text.StringBuilder sb, string line) => sb.Append(line).Append('\n');

        public static string Scalar(Kind kind) => kind switch
        {
            Kind.F32 => "f32",
            Kind.I32 => "i32",
            Kind.U32 => "u32",
            _ => "bool",
        };

        public static string Type(Kind kind, int width) => width == 1 ? Scalar(kind) : $"vec{width}<{Scalar(kind)}>";

        /// <summary>Constructor call that builds a vector (or scalar) of the given kind from component expressions.</summary>
        public static string Construct(Kind kind, IReadOnlyList<string> components) =>
            components.Count == 1 ? components[0] : $"{Type(kind, components.Count)}({string.Join(", ", components)})";

        public static string Splat(Kind kind, int width, string scalarLiteral) =>
            width == 1 ? scalarLiteral : $"{Type(kind, width)}({scalarLiteral})";

        public static string Swizzle(string expression, IReadOnlyList<int> components) =>
            expression + "." + string.Concat(components.Select(c => "xyzw"[c]));

        public static string IntLiteral(uint bits, bool signed)
        {
            if (!signed) return bits.ToString(CultureInfo.InvariantCulture) + "u";
            int value = unchecked((int)bits);
            if (value == int.MinValue) return "bitcast<i32>(0x80000000u)";
            return value < 0 ? $"({value.ToString(CultureInfo.InvariantCulture)}i)" : value.ToString(CultureInfo.InvariantCulture) + "i";
        }

        public static string FloatLiteral(float value)
        {
            string text = value.ToString("R", CultureInfo.InvariantCulture);
            if (!text.Contains('.') && !text.Contains('E')) text += ".0";
            else if (!text.Contains('.')) text = text.Replace("E", ".0E", StringComparison.Ordinal);
            return value < 0 || float.IsNegative(value) ? $"({text})" : text;
        }
    }
}
