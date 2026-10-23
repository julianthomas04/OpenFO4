using System.Globalization;
using System.Numerics;
using System.Reflection;
using System.Text.Json.Nodes;
using NiflySharp.Blocks;

namespace AssetConverter
{
    /// <summary>Raised when a shader block is present but NiflySharp's layout no longer yields its fields, or a field holds a value the extras cannot carry.</summary>
    public sealed class EffectShaderUnreadableException : Exception
    {
        public EffectShaderUnreadableException(string message) : base(message) { }

        public EffectShaderUnreadableException(string message, Exception inner) : base(message, inner) { }
    }

    /// <summary>Effect-shader fields of a .bgem material file, with the paths as authored.</summary>
    public sealed record EffectMaterialParams(
        string BaseTexture,
        string GreyscaleTexture,
        string EnvTexture,
        string NormalTexture,
        string EnvMaskTexture,
        string ReflectanceTexture,
        string LightingTexture,
        string EmitGradientTexture,
        BaseMaterialFile.AlphaBlendModeType Blend,
        bool AlphaTest,
        byte AlphaTestRef,
        bool ZTest,
        bool ZWrite,
        bool TwoSided,
        bool TileU,
        bool TileV,
        bool GreyscaleToPaletteColor,
        bool GreyscaleToPaletteAlpha,
        bool FalloffEnabled,
        bool FalloffColorEnabled,
        bool SoftEnabled,
        bool EffectLightingEnabled,
        bool EnvironmentMapping,
        Vector4 BaseColor,
        float BaseColorScale,
        float FalloffStartAngle,
        float FalloffStopAngle,
        float FalloffStartOpacity,
        float FalloffStopOpacity,
        float SoftDepth,
        float LightingInfluence,
        float EnvironmentMapScale,
        Vector3 EmittanceColor,
        float RefractionPower,
        float Alpha,
        Vector2 UvOffset,
        Vector2 UvScale)
    {
        public static EffectMaterialParams FromBgem(BGEM bgem) => new(
            BaseTexture: bgem.BaseTexture ?? "",
            GreyscaleTexture: bgem.GrayscaleTexture ?? "",
            EnvTexture: bgem.EnvmapTexture ?? "",
            NormalTexture: bgem.NormalTexture ?? "",
            EnvMaskTexture: bgem.EnvmapMaskTexture ?? "",
            ReflectanceTexture: bgem.SpecularTexture ?? "",
            LightingTexture: bgem.LightingTexture ?? "",
            EmitGradientTexture: bgem.GlowTexture ?? "",
            Blend: bgem.AlphaBlendMode,
            AlphaTest: bgem.AlphaTest,
            AlphaTestRef: bgem.AlphaTestRef,
            ZTest: bgem.ZBufferTest,
            ZWrite: bgem.ZBufferWrite,
            TwoSided: bgem.TwoSided,
            TileU: bgem.TileU,
            TileV: bgem.TileV,
            GreyscaleToPaletteColor: bgem.GrayscaleToPaletteColor,
            GreyscaleToPaletteAlpha: bgem.GrayscaleToPaletteAlpha,
            FalloffEnabled: bgem.FalloffEnabled,
            FalloffColorEnabled: bgem.FalloffColorEnabled,
            SoftEnabled: bgem.SoftEnabled,
            EffectLightingEnabled: bgem.EffectLightingEnabled,
            EnvironmentMapping: bgem.EnvironmentMapping,
            BaseColor: new Vector4(UnpackRgb(bgem.BaseColor), 1f),
            BaseColorScale: bgem.BaseColorScale,
            FalloffStartAngle: bgem.FalloffStartAngle,
            FalloffStopAngle: bgem.FalloffStopAngle,
            FalloffStartOpacity: bgem.FalloffStartOpacity,
            FalloffStopOpacity: bgem.FalloffStopOpacity,
            SoftDepth: bgem.SoftDepth,
            LightingInfluence: bgem.LightingInfluence,
            EnvironmentMapScale: bgem.EnvironmentMappingMaskScale,
            EmittanceColor: UnpackRgb(bgem.EmittanceColor),
            RefractionPower: bgem.RefractionPower,
            Alpha: bgem.Alpha,
            UvOffset: new Vector2(bgem.UOffset, bgem.VOffset),
            UvScale: new Vector2(bgem.UScale, bgem.VScale));

        private static Vector3 UnpackRgb(uint packed) =>
            new((packed >> 16 & 0xFF) / 255f, (packed >> 8 & 0xFF) / 255f, (packed & 0xFF) / 255f);
    }

    /// <summary>
    /// The fields of a NIF BSEffectShaderProperty. NiflySharp 0.1.0 exposes only the flag words and the UV transform
    /// publicly; the rest is read from the private fields named in <see cref="ReflectedFieldNames"/> (a test pins them
    /// against the real type). Texture paths are as authored.
    /// </summary>
    internal sealed record EffectShaderData(
        uint Flags1,
        uint Flags2,
        Vector2 UvOffset,
        Vector2 UvScale,
        string SourceTexture,
        string GreyscaleTexture,
        string EnvTexture,
        string NormalTexture,
        string EnvMaskTexture,
        string ReflectanceTexture,
        string LightingTexture,
        string EmitGradientTexture,
        byte ClampMode,
        byte LightingInfluence,
        float FalloffStartAngle,
        float FalloffStopAngle,
        float FalloffStartOpacity,
        float FalloffStopOpacity,
        float RefractionPower,
        Vector4 BaseColor,
        float BaseColorScale,
        float SoftFalloffDepth,
        float EnvironmentMapScale,
        Vector3 EmittanceColor)
    {
        public static readonly IReadOnlyList<string> ReflectedFieldNames =
        [
            "_sourceTexture", "_greyscaleTexture", "_envMapTexture", "_normalTexture", "_envMaskTexture", "_reflectanceTexture",
            "_lightingTexture", "_emitGradientTexture", "_textureClampMode", "_lightingInfluence", "_falloffStartAngle",
            "_falloffStopAngle", "_falloffStartOpacity", "_falloffStopOpacity", "_refractionPower", "_baseColor",
            "_baseColorScale", "_softFalloffDepth", "_environmentMapScale_fl", "_emittanceColor",
        ];

        private const BindingFlags PrivateInstance = BindingFlags.NonPublic | BindingFlags.Instance;

        public static EffectShaderData Read(BSEffectShaderProperty shader)
        {
            try
            {
                return new EffectShaderData(
                    Flags1: (uint)shader.ShaderFlags_F4SPF1,
                    Flags2: (uint)shader.ShaderFlags_F4SPF2,
                    UvOffset: new Vector2(shader.UVOffset.U, shader.UVOffset.V),
                    UvScale: new Vector2(shader.UVScale.U, shader.UVScale.V),
                    SourceTexture: Text(shader, "_sourceTexture"),
                    GreyscaleTexture: Text(shader, "_greyscaleTexture"),
                    EnvTexture: Text(shader, "_envMapTexture"),
                    NormalTexture: Text(shader, "_normalTexture"),
                    EnvMaskTexture: Text(shader, "_envMaskTexture"),
                    ReflectanceTexture: Text(shader, "_reflectanceTexture"),
                    LightingTexture: Text(shader, "_lightingTexture"),
                    EmitGradientTexture: Text(shader, "_emitGradientTexture"),
                    ClampMode: Convert.ToByte(Value(shader, "_textureClampMode"), CultureInfo.InvariantCulture),
                    LightingInfluence: Convert.ToByte(Value(shader, "_lightingInfluence"), CultureInfo.InvariantCulture),
                    FalloffStartAngle: Number(shader, "_falloffStartAngle"),
                    FalloffStopAngle: Number(shader, "_falloffStopAngle"),
                    FalloffStartOpacity: Number(shader, "_falloffStartOpacity"),
                    FalloffStopOpacity: Number(shader, "_falloffStopOpacity"),
                    RefractionPower: Number(shader, "_refractionPower"),
                    BaseColor: Color4(Value(shader, "_baseColor")),
                    BaseColorScale: Number(shader, "_baseColorScale"),
                    SoftFalloffDepth: Number(shader, "_softFalloffDepth"),
                    EnvironmentMapScale: Number(shader, "_environmentMapScale_fl"),
                    EmittanceColor: Color3(Value(shader, "_emittanceColor")));
            }
            catch (Exception ex) when (ReadFailureTally.IsReadFailure(ex))
            {
                throw new EffectShaderUnreadableException($"BSEffectShaderProperty fields could not be read: {ex.Message}", ex);
            }
        }

        /// <summary>The field's value; null is a legitimate value (an unset texture path), a missing field is not.</summary>
        internal static object? Field(object shader, string name)
        {
            var field = shader.GetType().GetField(name, PrivateInstance)
                ?? throw new InvalidDataException($"{shader.GetType().Name} has no field `{name}`");
            return field.GetValue(shader);
        }

        private static object Value(object shader, string name) =>
            Field(shader, name) ?? throw new InvalidDataException($"{shader.GetType().Name} field `{name}` is unset");

        internal static float Number(object shader, string name) => Convert.ToSingle(Value(shader, name), CultureInfo.InvariantCulture);

        internal static string Text(object shader, string name)
        {
            object? value = Field(shader, name);
            if (value == null)
                return "";
            var content = value.GetType().GetProperty("Content")
                ?? throw new InvalidDataException($"{value.GetType().Name} (field `{name}`) has no `Content` property");
            return content.GetValue(value) as string ?? "";
        }

        private static float Component(object color, string name) =>
            Convert.ToSingle(color.GetType().GetProperty(name)?.GetValue(color)
                ?? throw new InvalidDataException($"{color.GetType().Name} has no readable component `{name}`"), CultureInfo.InvariantCulture);

        private static Vector4 Color4(object color) => new(Component(color, "R"), Component(color, "G"), Component(color, "B"), Component(color, "A"));

        private static Vector3 Color3(object color) => new(Component(color, "R"), Component(color, "G"), Component(color, "B"));
    }

    /// <summary>Builds <c>material.extras.effect</c>: the permutation, pipeline state, textures, parameters and controllers of one effect-shader shape.</summary>
    internal static class EffectMaterialExtras
    {
        public const int Version = 1;
        public const float SoftDepthToEngineUnits = 0.1f;
        public const float AlphaTestScale = 1f / 255f;

        internal static readonly string[] BlendFactors =
        [
            "one", "zero", "src_color", "inv_src_color", "dst_color", "inv_dst_color", "src_alpha", "inv_src_alpha",
            "dst_alpha", "inv_dst_alpha", "src_alpha_saturate",
        ];

        internal static readonly string[] TestFunctions = ["always", "less", "equal", "lequal", "greater", "notequal", "gequal", "never"];

        internal static readonly string[] ClampModes = ["clamp_s_clamp_t", "clamp_s_wrap_t", "wrap_s_clamp_t", "wrap_s_wrap_t"];

        /// <param name="DeployTexture">Raw texture path to its deployed form (empty stays empty).</param>
        /// <param name="envTextureExists">Existence check of environment textures; defaults to the one the object-shader tokens use.</param>
        public static JsonObject Build(
            EffectShaderData shader,
            EffectAlphaState? alpha,
            EffectMaterialParams? material,
            EffectSelection selection,
            IReadOnlyList<ControllerTrack> controllers,
            Func<string, string> deployTexture,
            Func<string, bool>? envTextureExists = null)
        {
            if (selection.IsHidden)
                throw new ArgumentException($"A hidden effect shape has no extras ({selection.HiddenReason}).", nameof(selection));

            var blend = BlendObject(alpha, material);
            bool zTest = material?.ZTest ?? EffectShaderFlags.Has(shader.Flags1, EffectShaderFlags.ZTest);
            bool zWrite = material?.ZWrite ?? EffectShaderFlags.Has(shader.Flags2, EffectShaderFlags.ZWrite);
            bool doubleSided = material?.TwoSided ?? EffectShaderFlags.Has(shader.Flags2, EffectShaderFlags.DoubleSided);
            var textures = TexturesObject(shader, material, selection, deployTexture, envTextureExists ?? MaterialCensus.EnvTextureExists);
            var baseColor = material?.BaseColor ?? shader.BaseColor;
            var uvOffset = material?.UvOffset ?? shader.UvOffset;
            var uvScale = material?.UvScale ?? shader.UvScale;
            var emittance = material?.EmittanceColor ?? shader.EmittanceColor;

            return new JsonObject
            {
                ["v"] = Version,
                ["vs"] = selection.VertexName,
                ["ps"] = selection.PixelName,
                ["flags1"] = shader.Flags1,
                ["flags2"] = shader.Flags2,
                ["blend"] = blend,
                ["z_test"] = zTest,
                ["z_write"] = zWrite,
                ["double_sided"] = doubleSided,
                ["clamp"] = ClampModes[ClampIndex(shader, material)],
                ["textures"] = textures,
                ["base_color"] = Floats(baseColor.X, baseColor.Y, baseColor.Z, baseColor.W),
                ["base_color_scale"] = Finite(material?.BaseColorScale ?? shader.BaseColorScale, "base_color_scale"),
                ["falloff"] = new JsonObject
                {
                    ["start_angle"] = Finite(material?.FalloffStartAngle ?? shader.FalloffStartAngle, "falloff start angle"),
                    ["stop_angle"] = Finite(material?.FalloffStopAngle ?? shader.FalloffStopAngle, "falloff stop angle"),
                    ["start_opacity"] = Finite(material?.FalloffStartOpacity ?? shader.FalloffStartOpacity, "falloff start opacity"),
                    ["stop_opacity"] = Finite(material?.FalloffStopOpacity ?? shader.FalloffStopOpacity, "falloff stop opacity"),
                },
                ["soft_falloff_depth"] = Finite((material?.SoftDepth ?? shader.SoftFalloffDepth) * SoftDepthToEngineUnits, "soft falloff depth"),
                ["uv_offset"] = Floats(uvOffset.X, uvOffset.Y),
                ["uv_scale"] = Floats(uvScale.X, uvScale.Y),
                ["lighting_influence"] = Finite(EffectParams.ClampInfluence(material?.LightingInfluence ?? shader.LightingInfluence / 255f), "lighting influence"),
                ["env_map_scale"] = Finite(material?.EnvironmentMapScale ?? shader.EnvironmentMapScale, "env map scale"),
                ["emittance_color"] = Floats(emittance.X, emittance.Y, emittance.Z),
                ["refraction_power"] = Finite(material?.RefractionPower ?? shader.RefractionPower, "refraction power"),
                ["alpha"] = Finite(material?.Alpha ?? 1f, "alpha"),
                ["controllers"] = ControllerJson.Tracks(controllers),
            };
        }

        private static int ClampIndex(EffectShaderData shader, EffectMaterialParams? material)
        {
            if (material == null)
            {
                if (shader.ClampMode >= ClampModes.Length)
                    throw new EffectShaderUnreadableException($"Texture clamp mode {shader.ClampMode} is not 0..3.");
                return shader.ClampMode;
            }
            return (material.TileU ? 2 : 0) | (material.TileV ? 1 : 0);
        }

        private static JsonObject BlendObject(EffectAlphaState? alpha, EffectMaterialParams? material)
        {
            var (mode, source, destination) = BlendState(alpha, material);
            var blend = new JsonObject { ["mode"] = mode };
            if (mode != "none")
            {
                blend["src"] = source;
                blend["dst"] = destination;
            }

            bool test = material?.AlphaTest ?? alpha?.Test ?? false;
            blend["alpha_test"] = test;
            if (test)
            {
                int function = material != null ? 4 : alpha!.Value.TestFunction;
                byte reference = material?.AlphaTestRef ?? alpha!.Value.Threshold;
                blend["test_func"] = TestFunctions[function];
                blend["test_ref"] = reference * AlphaTestScale;
            }
            return blend;
        }

        private static (string Mode, string Source, string Destination) BlendState(EffectAlphaState? alpha, EffectMaterialParams? material)
        {
            if (material != null)
            {
                return EffectBlendModes.FromMaterial(material.Blend) switch
                {
                    EffectBlend.Add => ("add", "src_alpha", "one"),
                    EffectBlend.Mult => ("mult", "dst_color", "zero"),
                    EffectBlend.Alpha => ("alpha", "src_alpha", "inv_src_alpha"),
                    _ => ("none", "", ""),
                };
            }
            if (alpha is not { } state || state.Mode == EffectBlend.None) return ("none", "", "");
            string mode = state.Mode switch
            {
                EffectBlend.Add => "add",
                EffectBlend.Mult => "mult",
                _ => "alpha",
            };
            return (mode, Factor(state.SourceFactor), Factor(state.DestinationFactor));
        }

        private static string Factor(int value) =>
            value >= 0 && value < BlendFactors.Length
                ? BlendFactors[value]
                : throw new EffectShaderUnreadableException($"Blend factor {value} is not 0..10.");

        private static JsonObject TexturesObject(
            EffectShaderData shader,
            EffectMaterialParams? material,
            EffectSelection selection,
            Func<string, string> deploy,
            Func<string, bool> envTextureExists)
        {
            string Pick(string fromMaterial, string fromShader) => deploy(material != null ? fromMaterial : fromShader);
            var (env, envMask) = EnvironmentTextures(shader, material, selection, envTextureExists);
            return new JsonObject
            {
                ["source"] = Pick(material?.BaseTexture ?? "", shader.SourceTexture),
                ["greyscale"] = Pick(material?.GreyscaleTexture ?? "", shader.GreyscaleTexture),
                ["normal"] = Pick(material?.NormalTexture ?? "", shader.NormalTexture),
                ["env"] = deploy(env),
                ["env_mask"] = deploy(envMask),
                ["reflectance"] = Pick(material?.ReflectanceTexture ?? "", shader.ReflectanceTexture),
                ["lighting"] = Pick(material?.LightingTexture ?? "", shader.LightingTexture),
                ["emit_gradient"] = Pick(material?.EmitGradientTexture ?? "", shader.EmitGradientTexture),
            };
        }

        /// <summary>The raw environment cube and mask the extras carry. Only a permutation with the environment bit
        /// reads the cube, so only then it is resolved, through the same <see cref="EnvironmentParams.Resolve"/> the
        /// object-shader tokens use: a cube missing from the texture index is dropped (the engine then reflects black)
        /// and counted instead of leaving a dead reference. The mask is kept as authored when no cube resolves, since
        /// particle permutations read it independently of the cube.</summary>
        private static (string Cube, string Mask) EnvironmentTextures(
            EffectShaderData shader,
            EffectMaterialParams? material,
            EffectSelection selection,
            Func<string, bool> envTextureExists)
        {
            string cube = material != null ? material.EnvTexture : shader.EnvTexture;
            string mask = material != null ? material.EnvMaskTexture : shader.EnvMaskTexture;
            if ((selection.PixelDescriptor & EffectDescriptorBits.EnvMapping) == 0) return ("", mask);

            var resolved = EnvironmentParams.Resolve(true, cube, mask, 0f, envTextureExists, out bool cubeMissing);
            if (cubeMissing) NifAnimationStats.Add(EffectCounters.EnvironmentCubeUnresolved);
            return resolved == null ? ("", mask) : (resolved.CubeTexture, resolved.MaskTexture);
        }

        internal static float Finite(float value, string what) =>
            float.IsFinite(value) ? value : throw new EffectShaderUnreadableException($"{what} is not a finite number.");

        internal static JsonArray Floats(params float[] values)
        {
            var array = new JsonArray();
            foreach (float value in values) array.Add(Finite(value, "a vector component"));
            return array;
        }
    }

    internal static class ControllerJson
    {
        public static JsonArray Tracks(IReadOnlyList<ControllerTrack> tracks)
        {
            var array = new JsonArray();
            foreach (var track in tracks) array.Add(Track(track));
            return array;
        }

        private static JsonObject Track(ControllerTrack track)
        {
            var values = new JsonArray();
            foreach (var value in track.Values)
                values.Add(value.Length == 1 ? JsonValue.Create(EffectMaterialExtras.Finite(value[0], "a controller key")) : EffectMaterialExtras.Floats(value));
            var times = new JsonArray();
            foreach (float time in track.Times) times.Add(EffectMaterialExtras.Finite(time, "a controller time"));
            return new JsonObject
            {
                ["target"] = track.Target,
                ["cycle"] = track.Cycle,
                ["start"] = EffectMaterialExtras.Finite(track.Start, "a controller start"),
                ["stop"] = EffectMaterialExtras.Finite(track.Stop, "a controller stop"),
                ["frequency"] = EffectMaterialExtras.Finite(track.Frequency, "a controller frequency"),
                ["phase"] = EffectMaterialExtras.Finite(track.Phase, "a controller phase"),
                ["times"] = times,
                ["values"] = values,
            };
        }
    }

    /// <summary>The fields a refraction shape (a lighting-shader shape with the Refraction flag) hands the engine's heat-haze stage.</summary>
    internal sealed record RefractionShaderData(
        uint Flags1,
        uint Flags2,
        float Strength,
        Vector2 UvOffset,
        Vector2 UvScale,
        bool MeshHasVertexColors)
    {
        public const uint RefractionFlag = 0x8000;
        public static readonly IReadOnlyList<string> ReflectedFieldNames = ["_refractionStrength"];

        public bool VertexColors => EffectShaderFlags.Has(Flags2, EffectShaderFlags.VertexColors) && MeshHasVertexColors;

        public bool DoubleSided => EffectShaderFlags.Has(Flags2, EffectShaderFlags.DoubleSided);

        /// <summary>Null when the shader does not carry the Refraction flag; the private strength field is only read for shaders that do.</summary>
        public static RefractionShaderData? Read(BSLightingShaderProperty shader, bool meshHasVertexColors)
        {
            uint flags1 = (uint)shader.ShaderFlags_F4SPF1;
            if ((flags1 & RefractionFlag) == 0)
                return null;
            try
            {
                return new RefractionShaderData(
                    Flags1: flags1,
                    Flags2: (uint)shader.ShaderFlags_F4SPF2,
                    Strength: EffectShaderData.Number(shader, "_refractionStrength"),
                    UvOffset: new Vector2(shader.UVOffset.U, shader.UVOffset.V),
                    UvScale: new Vector2(shader.UVScale.U, shader.UVScale.V),
                    MeshHasVertexColors: meshHasVertexColors);
            }
            catch (Exception ex) when (ReadFailureTally.IsReadFailure(ex))
            {
                throw new EffectShaderUnreadableException($"BSLightingShaderProperty refraction fields could not be read: {ex.Message}", ex);
            }
        }
    }

    /// <summary>Builds <c>material.extras.refraction</c>.</summary>
    internal static class RefractionMaterialExtras
    {
        public static JsonObject Build(RefractionShaderData shader, string normalA, string normalB, IReadOnlyList<ControllerTrack> controllers) => new()
        {
            ["v"] = EffectMaterialExtras.Version,
            ["strength"] = EffectMaterialExtras.Finite(shader.Strength, "refraction strength"),
            ["normal_a"] = normalA,
            ["normal_b"] = normalB,
            ["uv_offset"] = EffectMaterialExtras.Floats(shader.UvOffset.X, shader.UvOffset.Y),
            ["uv_scale"] = EffectMaterialExtras.Floats(shader.UvScale.X, shader.UvScale.Y),
            ["vertex_colors"] = shader.VertexColors,
            ["double_sided"] = shader.DoubleSided,
            ["controllers"] = ControllerJson.Tracks(controllers),
        };
    }

    internal static class EffectCounters
    {
        public const string WithExtras = "effect shapes drawn through effect extras";
        public const string HiddenPrefix = "effect shapes left hidden: ";
        public const string RefractionShapes = "refraction shapes (refr token and extras)";
        public const string RefractionSkippedPrefix = "refraction shapes left as plain lighting shapes: ";
        public const string NoCatalogReason = "no shader catalogue loaded";
        public const string UnreadableReason = "unreadable shader or controller data";
        public const string ProceduralLightningReason = "procedural lightning";
        public const string EnvironmentCubeUnresolved = "effect shapes whose environment cube is missing from the texture index (extras reflect black)";
    }

    /// <summary>Converts one effect-shader shape: the extras to attach, or null when the shape keeps the object-shader effect path (counted by reason).</summary>
    internal static class EffectShapeConversion
    {
        /// <summary>The shader fields, the single read of a BSEffectShaderProperty: the extras and the object-shader name tokens derive from them. Null (counted) when they cannot be read.</summary>
        /// <param name="reportWarning">Receives the detail of a shape that could not be read; the counters carry only the fixed reason.</param>
        public static EffectShaderData? TryRead(BSTriShape shape, BSEffectShaderProperty shader, Action<string> reportWarning)
        {
            try
            {
                return EffectShaderData.Read(shader);
            }
            catch (EffectShaderUnreadableException ex)
            {
                reportWarning($"effect shape '{shape.Name?.String}' left hidden: {ex.Message}");
                Hidden(EffectCounters.UnreadableReason);
                return null;
            }
        }

        /// <param name="data">The shape's shader fields from <see cref="TryRead"/>; null skips the extras.</param>
        /// <param name="reportWarning">Receives the detail of a shape that could not be read; the counters carry only the fixed reason.</param>
        public static JsonObject? TryBuild(
            BSTriShape shape,
            NiflySharp.NifFile nif,
            int shaderBlock,
            BSEffectShaderProperty shader,
            EffectShaderData? data,
            MaterialDef? materialDef,
            IEffectShaderCatalog? catalog,
            Func<string, string> deployTexture,
            Action<string> reportWarning)
        {
            if (data == null || shape.VertexPositions is not { Count: > 0 }) return null;
            if (catalog == null) return Hidden(EffectCounters.NoCatalogReason);

            try
            {
                if (HasProceduralLightning(nif, shaderBlock, shader)) return Hidden(EffectCounters.ProceduralLightningReason);
                var material = materialDef?.Effect;
                var alpha = ReadAlpha(shape, nif);
                var facts = EffectShapeFacts.Create(data, alpha, material, EffectGeometry.OfMesh(VertexFlags(shape), shape.SkinInstanceRef.Index >= 0));
                var selection = EffectTechnique.Select(facts, catalog);
                if (selection.IsHidden) return Hidden(selection.HiddenReason!);

                var tracks = ShaderControllerTracks.ForShader(nif, shaderBlock).Tracks;
                var extras = EffectMaterialExtras.Build(data, alpha, material, selection, tracks, deployTexture);
                NifAnimationStats.Add(EffectCounters.WithExtras);
                return extras;
            }
            catch (Exception ex) when (ex is EffectShaderUnreadableException or NifAnimationException)
            {
                reportWarning($"effect shape '{shape.Name?.String}' left hidden: {ex.Message}");
                return Hidden(EffectCounters.UnreadableReason);
            }
        }

        public static uint VertexFlags(BSTriShape shape)
        {
            uint flags = 0;
            if (shape.HasUVs) flags |= EffectVertexFlags.Uv;
            if (shape.HasNormals) flags |= EffectVertexFlags.Normal;
            if (shape.HasTangents) flags |= EffectVertexFlags.Tangent;
            if (shape.HasVertexColors) flags |= EffectVertexFlags.Color;
            if (shape.IsSkinned) flags |= EffectVertexFlags.Skinned;
            return flags;
        }

        public static EffectAlphaState? ReadAlpha(BSTriShape shape, NiflySharp.NifFile nif) =>
            ReadAlpha(nif, shape.HasAlphaProperty, shape.AlphaPropertyRef.Index);

        public static EffectAlphaState? ReadAlpha(NiflySharp.NifFile nif, bool hasAlphaProperty, int index)
        {
            if (!hasAlphaProperty) return null;
            if (index < 0 || index >= nif.Blocks.Count || nif.Blocks[index] is not NiAlphaProperty property) return null;
            return EffectAlphaState.FromFlags(property.Flags.Value, property.Threshold);
        }

        // The bolt generator is a controller on a node that names the shader it draws with; a controller on the shader itself counts too.
        private static bool HasProceduralLightning(NiflySharp.NifFile nif, int shaderBlock, BSEffectShaderProperty shader) =>
            nif.Blocks.OfType<BSProceduralLightningController>().Any(c => c.ShaderProperty.Index == shaderBlock)
            || ShaderControllerTracks.ControllerChain(nif, shader).Any(c => c.Controller is BSProceduralLightningController);

        private static JsonObject? Hidden(string reason)
        {
            NifAnimationStats.Add(EffectCounters.HiddenPrefix + reason);
            return null;
        }
    }

    /// <summary>Refraction shapes: lighting-shader shapes with the Refraction flag.</summary>
    internal static class RefractionShapeConversion
    {
        public static JsonObject? TryBuild(
            BSTriShape shape,
            NiflySharp.NifFile nif,
            int shaderBlock,
            BSLightingShaderProperty shader,
            string normalA,
            string normalB,
            Action<string> reportWarning)
        {
            if (shape.VertexPositions is not { Count: > 0 }) return null;
            try
            {
                var data = RefractionShaderData.Read(shader, shape.HasVertexColors);
                if (data == null) return null;
                if (shape.IsSkinned || shape.SkinInstanceRef.Index >= 0) return Skipped("skinned");

                var tracks = ShaderControllerTracks.ForShader(nif, shaderBlock).Tracks;
                var extras = RefractionMaterialExtras.Build(data, normalA, normalB, tracks);
                NifAnimationStats.Add(EffectCounters.RefractionShapes);
                return extras;
            }
            catch (Exception ex) when (ex is EffectShaderUnreadableException or NifAnimationException)
            {
                reportWarning($"refraction shape '{shape.Name?.String}' left as a plain lighting shape: {ex.Message}");
                return Skipped(EffectCounters.UnreadableReason);
            }
        }

        private static JsonObject? Skipped(string reason)
        {
            NifAnimationStats.Add(EffectCounters.RefractionSkippedPrefix + reason);
            return null;
        }
    }
}
