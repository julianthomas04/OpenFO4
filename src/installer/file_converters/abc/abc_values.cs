using System.Globalization;
using System.Text.RegularExpressions;

namespace AssetConverter.Abc
{
    /// <summary>
    /// ActionScript 3 semantics for the constant values the bytecode pushes (int, uint, Number, String, Boolean): text form for
    /// signatures, the conversions the conversion opcodes apply, and folding of operators whose operands are all constants.
    /// A result that is not a finite number, a string or a Boolean is not folded, and neither is a conversion whose ActionScript
    /// result is not certain; both leave the value computed.
    /// </summary>
    internal static partial class AbcValues
    {
        /// <summary>Marks an operand whose value cannot be described; <see cref="Describe"/> escapes it inside strings.</summary>
        public const char Undescribable = '';

        public static string Describe(object value) => value switch
        {
            string text => $"\"{text.Replace(Undescribable.ToString(), "\\uE000", StringComparison.Ordinal)}\"",
            bool flag => flag ? "true" : "false",
            double number => number.ToString("R", CultureInfo.InvariantCulture),
            IFormattable formattable => formattable.ToString(null, CultureInfo.InvariantCulture),
            _ => throw new ArgumentException($"{value.GetType().Name} is not an ActionScript constant.", nameof(value)),
        };

        public static bool IsNumber(object value) => value is int or uint or double;

        public static double ToNumber(object value) => value switch
        {
            int i => i,
            uint u => u,
            double d => d,
            _ => throw new InvalidOperationException($"{value} is not a number."),
        };

        public static int ToInt32(double value)
        {
            if (double.IsNaN(value) || double.IsInfinity(value))
                return 0;
            double truncated = Math.Truncate(value);
            double modulo = truncated % 4294967296.0;
            if (modulo < 0)
                modulo += 4294967296.0;
            return unchecked((int)(uint)modulo);
        }

        public static uint ToUInt32(double value) => unchecked((uint)ToInt32(value));

        /// <summary>
        /// The value after a conversion opcode (<c>convert_*</c>, <c>coerce_*</c>, <c>coerce</c> or <c>astype</c> to
        /// <paramref name="typeName"/>), or <c>null</c> when the result is not a constant this reader can state with certainty.
        /// </summary>
        public static object? Convert(string mnemonic, string? typeName, object value) => mnemonic switch
        {
            "convert_i" or "coerce_i" => ToNumberExact(value) is { } number ? ToInt32(number) : null,
            "convert_u" or "coerce_u" => ToNumberExact(value) is { } number ? ToUInt32(number) : null,
            "convert_d" or "coerce_d" => ToNumberExact(value) is { } number && double.IsFinite(number) ? number : null,
            "convert_s" or "coerce_s" => ToActionScriptString(value),
            "convert_b" or "coerce_b" => ToBoolean(value),
            "convert_o" or "coerce_a" or "coerce_o" => value,
            "coerce" => Coerce(typeName, value),
            "astype" => IsOfType(typeName, value) ? value : null,
            _ => null,
        };

        private static object? Coerce(string? typeName, object value) => typeName switch
        {
            "int" => Convert("convert_i", null, value),
            "uint" => Convert("convert_u", null, value),
            "Number" => Convert("convert_d", null, value),
            "String" => ToActionScriptString(value),
            "Boolean" => ToBoolean(value),
            "Object" or "*" => value,
            _ => null,
        };

        private static bool IsOfType(string? typeName, object value) => typeName switch
        {
            "Object" or "*" => true,
            "String" => value is string,
            "Boolean" => value is bool,
            "Number" => IsNumber(value),
            "int" => IsNumber(value) && ToNumber(value) is var n && n == Math.Truncate(n) && n >= int.MinValue && n <= int.MaxValue,
            "uint" => IsNumber(value) && ToNumber(value) is var n && n == Math.Truncate(n) && n >= 0 && n <= uint.MaxValue,
            _ => false,
        };

        public static bool ToBoolean(object value) => value switch
        {
            bool flag => flag,
            string text => text.Length > 0,
            _ when IsNumber(value) => ToNumber(value) is var n && n != 0 && !double.IsNaN(n),
            _ => throw new ArgumentException($"{value.GetType().Name} is not an ActionScript constant.", nameof(value)),
        };

        /// <summary>ActionScript's <c>String(value)</c> for a constant.</summary>
        public static string ToActionScriptString(object value) => value switch
        {
            string text => text,
            bool flag => flag ? "true" : "false",
            int i => i.ToString(CultureInfo.InvariantCulture),
            uint u => u.ToString(CultureInfo.InvariantCulture),
            double d => NumberToString(d),
            _ => throw new ArgumentException($"{value.GetType().Name} is not an ActionScript constant.", nameof(value)),
        };

        /// <summary>The ECMAScript Number-to-String conversion ActionScript uses: shortest round-trip digits, exponent form outside 1e-7..1e21.</summary>
        public static string NumberToString(double value)
        {
            if (double.IsNaN(value))
                return "NaN";
            if (double.IsInfinity(value))
                return value > 0 ? "Infinity" : "-Infinity";
            if (value == 0)
                return "0";
            string sign = value < 0 ? "-" : "";
            (string digits, int exponent) = DigitsAndExponent(Math.Abs(value).ToString("R", CultureInfo.InvariantCulture));
            int k = digits.Length;
            int n = exponent + 1;
            if (k <= n && n <= 21)
                return sign + digits + new string('0', n - k);
            if (0 < n && n <= 21)
                return sign + digits[..n] + "." + digits[n..];
            if (-6 < n && n <= 0)
                return sign + "0." + new string('0', -n) + digits;
            int e = n - 1;
            string mantissa = k == 1 ? digits : digits[..1] + "." + digits[1..];
            return sign + mantissa + "e" + (e < 0 ? "-" : "+") + Math.Abs(e).ToString(CultureInfo.InvariantCulture);
        }

        /// <summary>The significant digits (no leading or trailing zeros) and the decimal exponent of the first digit.</summary>
        private static (string Digits, int Exponent) DigitsAndExponent(string formatted)
        {
            int e = formatted.IndexOfAny(['E', 'e']);
            string mantissa = e < 0 ? formatted : formatted[..e];
            int exponent = e < 0 ? 0 : int.Parse(formatted[(e + 1)..], NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture);
            int point = mantissa.IndexOf('.', StringComparison.Ordinal);
            string all = mantissa.Replace(".", "", StringComparison.Ordinal);
            int integerDigits = point < 0 ? mantissa.Length : point;
            int leadingZeros = all.Length - all.TrimStart('0').Length;
            string digits = all.Trim('0');
            return (digits, exponent + integerDigits - 1 - leadingZeros);
        }

        /// <summary>ActionScript's <c>Number(value)</c> for a constant, or <c>null</c> for a string whose conversion this reader does not state.</summary>
        private static double? ToNumberExact(object value) => value switch
        {
            bool flag => flag ? 1 : 0,
            string text => StringToNumber(text),
            _ when IsNumber(value) => ToNumber(value),
            _ => null,
        };

        [GeneratedRegex(@"^[+-]?(Infinity|[0-9]+(\.[0-9]*)?([eE][+-]?[0-9]+)?|\.[0-9]+([eE][+-]?[0-9]+)?)$")]
        private static partial Regex DecimalLiteral();

        [GeneratedRegex(@"^0[xX][0-9a-fA-F]+$")]
        private static partial Regex HexLiteral();

        [GeneratedRegex(@"^[+-]0[xX]")]
        private static partial Regex SignedHexPrefix();

        /// <summary>
        /// The ECMAScript string-to-number grammar for the plain cases: empty is 0, an unsigned hexadecimal or a decimal literal is its value,
        /// anything else is NaN. Surrounding white space and a signed hexadecimal are left unstated.
        /// </summary>
        private static double? StringToNumber(string text)
        {
            if (text.Length == 0)
                return 0;
            if (text.Trim().Length != text.Length || SignedHexPrefix().IsMatch(text))
                return null;
            if (HexLiteral().IsMatch(text))
                return text.Length > 15 ? null : System.Convert.ToInt64(text[2..], 16);
            if (DecimalLiteral().IsMatch(text))
                return text.EndsWith("Infinity", StringComparison.Ordinal)
                    ? text.StartsWith('-') ? double.NegativeInfinity : double.PositiveInfinity
                    : double.Parse(text, NumberStyles.Float, CultureInfo.InvariantCulture);
            return double.NaN;
        }

        public static object? FoldUnary(string mnemonic, object value)
        {
            switch (mnemonic)
            {
                case "not":
                    return !ToBoolean(value);
                case "typeof":
                    return value switch { string => "string", bool => "boolean", _ => "number" };
            }
            if (ToNumberExact(value) is not { } number)
                return null;
            return mnemonic switch
            {
                "negate" => Finite(-number),
                "negate_i" => unchecked(-ToInt32(number)),
                "bitnot" => ~ToInt32(number),
                "increment" => Finite(number + 1),
                "decrement" => Finite(number - 1),
                "increment_i" => unchecked(ToInt32(number) + 1),
                "decrement_i" => unchecked(ToInt32(number) - 1),
                _ => null,
            };
        }

        public static object? FoldBinary(string mnemonic, object left, object right)
        {
            if (mnemonic == "add" && left is string leftText && right is string rightText)
                return leftText + rightText;
            if (!IsNumber(left) || !IsNumber(right))
                return null;
            double a = ToNumber(left);
            double b = ToNumber(right);
            int shift = (int)(ToUInt32(b) & 31);
            return mnemonic switch
            {
                "add" => Finite(a + b),
                "subtract" => Finite(a - b),
                "multiply" => Finite(a * b),
                "divide" => Finite(a / b),
                "modulo" => Finite(a % b),
                "lshift" => ToInt32(a) << shift,
                "rshift" => ToInt32(a) >> shift,
                "urshift" => ToUInt32(a) >> shift,
                "bitand" => ToInt32(a) & ToInt32(b),
                "bitor" => ToInt32(a) | ToInt32(b),
                "bitxor" => ToInt32(a) ^ ToInt32(b),
                "add_i" => unchecked(ToInt32(a) + ToInt32(b)),
                "subtract_i" => unchecked(ToInt32(a) - ToInt32(b)),
                "multiply_i" => unchecked(ToInt32(a) * ToInt32(b)),
                _ => null,
            };
        }

        private static object? Finite(double value) => double.IsFinite(value) ? value : null;
    }
}
