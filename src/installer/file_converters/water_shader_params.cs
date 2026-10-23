using System.Globalization;
using System.Reflection;
using NiflySharp.Blocks;

namespace AssetConverter
{
    /// <summary>Raised when a BSWaterShaderProperty is present but NiflySharp's layout no longer yields its fields.</summary>
    public sealed class WaterShaderUnreadableException(string message, Exception inner) : Exception(message, inner);

    /// <summary>
    /// The fields of a NIF BSWaterShaderProperty (the shader of placed water meshes). NiflySharp 0.1.0 exposes none of
    /// them through a public member (its public <c>ShaderFlags</c> / <c>ShaderFlags2</c> are the FO3/NV flag enums and
    /// there is no UV accessor), so they are read from the private fields named in <see cref="ReflectedFieldNames"/>;
    /// a test pins those names against the real type.
    /// <paramref name="WaterFlags"/> is the raw WaterShaderPropertyFlags value (bit 0 DISPLACEMENT, 1 LOD, 2 DEPTH,
    /// 3 ACTOR_IN_WATER, 4 ACTOR_IN_WATER_IS_MOVING, 5 UNDERWATER, 6 REFLECTIONS, 7 REFRACTIONS, 8 VERTEX_UV,
    /// 9 VERTEX_ALPHA_DEPTH, 10 PROCEDURAL, 11 FOG, 12 UPDATE_CONSTANTS, 13 CUBEMAP); the shader flags are the raw
    /// Skyrim/FO4 BSShaderProperty flag words 1 and 2.
    /// </summary>
    public sealed record WaterShaderParams(
        uint WaterFlags,
        uint ShaderFlags1,
        uint ShaderFlags2,
        float UvOffsetU,
        float UvOffsetV,
        float UvScaleU,
        float UvScaleV)
    {
        public const string TokenKey = "water";

        public const string WaterFlagsField = "_waterShaderFlags";
        public const string ShaderFlags1Field = "_shaderFlags1";
        public const string ShaderFlags2Field = "_shaderFlags2_SSPF2";
        public const string UvOffsetField = "_uVOffset";
        public const string UvScaleField = "_uVScale";
        public const string TexCoordU = "U";
        public const string TexCoordV = "V";

        public static readonly IReadOnlyList<string> ReflectedFieldNames =
            [WaterFlagsField, ShaderFlags1Field, ShaderFlags2Field, UvOffsetField, UvScaleField];

        const BindingFlags PrivateInstance = BindingFlags.NonPublic | BindingFlags.Instance;

        /// <summary>Reads the shader's fields; throws <see cref="WaterShaderUnreadableException"/> when NiflySharp's layout changed.</summary>
        public static WaterShaderParams FromShader(BSWaterShaderProperty shader) => Read(shader, typeof(BSWaterShaderProperty));

        internal static WaterShaderParams Read(object shader, Type layout)
        {
            try
            {
                return new WaterShaderParams(
                    WaterFlags: ReadFlags(shader, layout, WaterFlagsField),
                    ShaderFlags1: ReadFlags(shader, layout, ShaderFlags1Field),
                    ShaderFlags2: ReadFlags(shader, layout, ShaderFlags2Field),
                    UvOffsetU: ReadTexCoord(shader, layout, UvOffsetField, TexCoordU),
                    UvOffsetV: ReadTexCoord(shader, layout, UvOffsetField, TexCoordV),
                    UvScaleU: ReadTexCoord(shader, layout, UvScaleField, TexCoordU),
                    UvScaleV: ReadTexCoord(shader, layout, UvScaleField, TexCoordV));
            }
            catch (Exception ex) when (ReadFailureTally.IsReadFailure(ex))
            {
                throw new WaterShaderUnreadableException($"{layout.Name} fields could not be read: {ex.Message}", ex);
            }
        }

        /// <summary>Material-name token: <c>water:WF:F1:F2:UOU,UOV:USU,USV</c> (WF decimal, F1 and F2 8-digit hex, floats round-trip).</summary>
        public string ToToken() => string.Create(CultureInfo.InvariantCulture,
            $"{TokenKey}:{WaterFlags}:{ShaderFlags1:X8}:{ShaderFlags2:X8}:{Float(UvOffsetU)},{Float(UvOffsetV)}:{Float(UvScaleU)},{Float(UvScaleV)}");

        static string Float(float value) => value.ToString("R", CultureInfo.InvariantCulture);

        static object Field(object shader, Type layout, string name) =>
            layout.GetField(name, PrivateInstance)?.GetValue(shader)
            ?? throw new InvalidDataException($"{layout.Name} has no readable field `{name}`");

        static uint ReadFlags(object shader, Type layout, string name) =>
            Convert.ToUInt32(Field(shader, layout, name), CultureInfo.InvariantCulture);

        static float ReadTexCoord(object shader, Type layout, string name, string component)
        {
            object coord = Field(shader, layout, name);
            object value = coord.GetType().GetProperty(component)?.GetValue(coord)
                ?? throw new InvalidDataException($"TexCoord has no readable component `{component}`");
            return Convert.ToSingle(value, CultureInfo.InvariantCulture);
        }
    }
}
