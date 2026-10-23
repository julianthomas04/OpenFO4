using System.Globalization;
using AssetConverter.ShaderTranslation.Archive;
using AssetConverter.ShaderTranslation.Dxbc;
using NiflySharp.Blocks;

namespace AssetConverter
{
    internal enum EffectBlend
    {
        None,
        Alpha,
        Add,
        Mult,
    }

    internal static class EffectBlendModes
    {
        public static EffectBlend FromMaterial(BaseMaterialFile.AlphaBlendModeType mode) => mode switch
        {
            BaseMaterialFile.AlphaBlendModeType.Additive => EffectBlend.Add,
            BaseMaterialFile.AlphaBlendModeType.Multiplicative => EffectBlend.Mult,
            BaseMaterialFile.AlphaBlendModeType.Standard => EffectBlend.Alpha,
            _ => EffectBlend.None,
        };
    }

    /// <summary>Bits of the effect shader property's two flag words that the converter reads.</summary>
    internal static class EffectShaderFlags
    {
        public const uint PaletteColor = 1u << 4;
        public const uint PaletteAlpha = 1u << 5;
        public const uint Falloff = 1u << 6;
        public const uint EnvironmentMapping = 1u << 7;
        public const uint ColorFalloff = 1u << 8;
        public const uint Decal = 1u << 26;
        public const uint SoftDepth = 1u << 30;
        public const uint ZTest = 1u << 31;

        public const uint ZWrite = 1u << 0;
        public const uint DoubleSided = 1u << 4;
        public const uint VertexColors = 1u << 5;
        public const uint EffectLighting = 1u << 30;

        public static bool Has(uint flags, uint flag) => (flags & flag) != 0;
    }

    /// <summary>NiAlphaProperty state: blend / test switches, blend factors (NIF enumeration values) and test function.</summary>
    internal readonly record struct EffectAlphaState(bool Blend, int SourceFactor, int DestinationFactor, bool Test, int TestFunction, byte Threshold)
    {
        private const int OneFactor = 0;
        private const int ZeroFactor = 1;
        private const int SourceColorFactor = 2;
        private const int DestinationColorFactor = 4;

        /// <summary>Unpacks the raw flag word: bit 0 blend, bits 1-4 source factor, bits 5-8 destination factor, bit 9 test, bits 10-12 test function.</summary>
        public static EffectAlphaState FromFlags(int flags, byte threshold) =>
            new((flags & 1) != 0, (flags >> 1) & 15, (flags >> 5) & 15, (flags >> 9 & 1) != 0, (flags >> 10) & 7, threshold);

        public EffectBlend Mode
        {
            get
            {
                if (!Blend) return EffectBlend.None;
                if (DestinationFactor == OneFactor) return EffectBlend.Add;
                bool multiplicative = (SourceFactor, DestinationFactor) is (ZeroFactor, SourceColorFactor) or (DestinationColorFactor, ZeroFactor);
                return multiplicative ? EffectBlend.Mult : EffectBlend.Alpha;
            }
        }
    }

    /// <summary>Everything the technique reads from one effect-shader shape; material-file values take precedence when <see cref="Material"/> is set.</summary>
    /// <param name="MeshVertexFlags">Vertex description attribute flags (uv 0x2, normal 0x8, tangent 0x10, colour 0x20, skinned 0x40); ignored for particles.</param>
    /// <param name="ParticleSubtextures">Sub-texture rectangle count of the particle data.</param>
    internal sealed record EffectShapeFacts(
        uint Flags1,
        uint Flags2,
        bool Particle,
        bool ParticleVertexColors,
        int ParticleSubtextures,
        uint MeshVertexFlags,
        bool HasSkinInstance,
        EffectAlphaState? Alpha,
        string SourceTexture,
        string GreyscaleTexture,
        string NormalTexture,
        string EnvMaskTexture,
        EffectMaterialParams? Material)
    {
        public static EffectShapeFacts Create(EffectShaderData data, EffectAlphaState? alpha, EffectMaterialParams? material, EffectGeometry geometry) => new(
            Flags1: data.Flags1,
            Flags2: data.Flags2,
            Particle: geometry.Particle,
            ParticleVertexColors: geometry.ParticleVertexColors,
            ParticleSubtextures: geometry.ParticleSubtextures,
            MeshVertexFlags: geometry.MeshVertexFlags,
            HasSkinInstance: geometry.HasSkinInstance,
            Alpha: alpha,
            SourceTexture: data.SourceTexture,
            GreyscaleTexture: data.GreyscaleTexture,
            NormalTexture: data.NormalTexture,
            EnvMaskTexture: data.EnvMaskTexture,
            Material: material);
    }

    /// <summary>The geometry-side inputs of <see cref="EffectShapeFacts"/>: a mesh's vertex attributes or a particle system's data.</summary>
    internal readonly record struct EffectGeometry(bool Particle, bool ParticleVertexColors, int ParticleSubtextures, uint MeshVertexFlags, bool HasSkinInstance)
    {
        public static EffectGeometry OfMesh(uint vertexFlags, bool hasSkinInstance) => new(false, false, 0, vertexFlags, hasSkinInstance);

        public static EffectGeometry OfParticles(NiPSysData data) => new(
            Particle: true,
            ParticleVertexColors: data.HasVertexColors == true,
            ParticleSubtextures: data.HasTextureIndices == true ? data.SubtextureOffsets?.Count ?? 0 : 0,
            MeshVertexFlags: 0,
            HasSkinInstance: false);
    }

    /// <summary>The derived switches of <see cref="EffectTechnique.Features"/>.</summary>
    internal sealed record EffectFeatures(
        bool Particle,
        bool VertexColors,
        bool Skinned,
        bool Uv,
        bool MeshNormals,
        bool Subtextures,
        EffectBlend Blend,
        bool PaletteColor,
        bool PaletteAlpha,
        bool Falloff,
        bool Soft,
        bool Lit,
        bool EnvMapping,
        bool ColorFalloff,
        bool Decal,
        bool ParticleNormalMask);

    internal static class EffectDescriptorBits
    {
        public const uint VertexColors = 0x1;
        public const uint Skinned = 0x2;
        public const uint Uv = 0x4;
        public const uint Subtextures = 0x8;
        public const uint Falloff = 0x10;
        public const uint Additive = 0x20;
        public const uint Multiplicative = 0x40;
        public const uint Particle = 0x80;
        public const uint Lighting = 0x400;
        public const uint SoftDepth = 0x1000;
        public const uint PaletteColor = 0x2000;
        public const uint PaletteAlpha = 0x4000;
        public const uint EnvMapping = 0x80000;
        public const uint ColorFalloff = 0x200000;
        public const uint ParticleNormalMask = 0x400000;
        public const uint Normals = 0x800000;
        public const uint Decal = 0x10000000;
        public const uint PixelOnly = 0x4800E000;
        public const uint VertexIrrelevant = Additive | Multiplicative | Decal;
    }

    /// <summary>Vertex description attribute flags of a mesh (the vertex description's top bits).</summary>
    internal static class EffectVertexFlags
    {
        public const uint Uv = 0x2;
        public const uint Normal = 0x8;
        public const uint Tangent = 0x10;
        public const uint Color = 0x20;
        public const uint Skinned = 0x40;
    }

    internal static class EffectShaderNames
    {
        public static string Vertex(uint descriptor) => $"effect_vs_{descriptor.ToString("x8", CultureInfo.InvariantCulture)}";

        public static string Pixel(uint descriptor) => $"effect_ps_{descriptor.ToString("x8", CultureInfo.InvariantCulture)}";
    }

    /// <summary>The effect shader family's descriptor set: which vertex / pixel permutations exist and whether a pair can be linked.</summary>
    internal interface IEffectShaderCatalog
    {
        bool HasVertexShader(uint descriptor);

        bool HasPixelShader(uint descriptor);

        /// <summary>Null when the pair links and the vertex shader's inputs are all in <paramref name="providedAttributes"/> (<c>SEMANTIC</c> + index, e.g. <c>TEXCOORD0</c>); otherwise the reason.</summary>
        string? CheckPair(uint vertexDescriptor, uint pixelDescriptor, IReadOnlySet<string> providedAttributes);
    }

    /// <summary>Outcome of permutation selection: the shader names, or the reason the shape stays hidden.</summary>
    internal sealed record EffectSelection(uint VertexDescriptor, uint PixelDescriptor, string? HiddenReason)
    {
        public bool IsHidden => HiddenReason != null;

        public string VertexName => EffectShaderNames.Vertex(VertexDescriptor);

        public string PixelName => EffectShaderNames.Pixel(PixelDescriptor);

        public static EffectSelection Hidden(uint pixelDescriptor, string reason) => new(0, pixelDescriptor, reason);
    }

    /// <summary>Picks the effect shader permutation of a shape from its shader flags, alpha property, textures, geometry and material file.</summary>
    internal static class EffectTechnique
    {
        public const string SkinnedReason = "skinned";
        public const string NoVerticesReason = "no vertices";
        public const string NoPixelShaderReason = "no pixel shader";
        public const string NoVertexShaderReason = "no vertex shader";
        public const string SignatureMismatchReason = "signature mismatch";

        public static EffectFeatures Features(EffectShapeFacts facts)
        {
            var material = facts.Material;
            EffectBlend blend;
            bool paletteColor, paletteAlpha, falloff, soft, lit, envMapping, colorFalloff;
            string source, greyscale, normal, envMask;
            if (material != null)
            {
                blend = EffectBlendModes.FromMaterial(material.Blend);
                paletteColor = material.GreyscaleToPaletteColor;
                paletteAlpha = material.GreyscaleToPaletteAlpha;
                falloff = material.FalloffEnabled;
                soft = material.SoftEnabled;
                lit = material.EffectLightingEnabled;
                envMapping = material.EnvironmentMapping;
                colorFalloff = material.FalloffColorEnabled;
                source = material.BaseTexture;
                greyscale = material.GreyscaleTexture;
                normal = material.NormalTexture;
                envMask = material.EnvMaskTexture;
            }
            else
            {
                blend = facts.Alpha?.Mode ?? EffectBlend.None;
                paletteColor = EffectShaderFlags.Has(facts.Flags1, EffectShaderFlags.PaletteColor);
                paletteAlpha = EffectShaderFlags.Has(facts.Flags1, EffectShaderFlags.PaletteAlpha);
                falloff = EffectShaderFlags.Has(facts.Flags1, EffectShaderFlags.Falloff);
                envMapping = EffectShaderFlags.Has(facts.Flags1, EffectShaderFlags.EnvironmentMapping);
                colorFalloff = EffectShaderFlags.Has(facts.Flags1, EffectShaderFlags.ColorFalloff);
                soft = EffectShaderFlags.Has(facts.Flags1, EffectShaderFlags.SoftDepth);
                lit = EffectShaderFlags.Has(facts.Flags2, EffectShaderFlags.EffectLighting);
                source = facts.SourceTexture;
                greyscale = facts.GreyscaleTexture;
                normal = facts.NormalTexture;
                envMask = facts.EnvMaskTexture;
            }

            bool particle = facts.Particle;
            bool meshHasUv = (facts.MeshVertexFlags & EffectVertexFlags.Uv) != 0;
            bool meshHasColors = (facts.MeshVertexFlags & EffectVertexFlags.Color) != 0;
            bool greyscalePresent = greyscale.Length > 0;
            return new EffectFeatures(
                Particle: particle,
                VertexColors: EffectShaderFlags.Has(facts.Flags2, EffectShaderFlags.VertexColors) && (particle ? facts.ParticleVertexColors : meshHasColors),
                Skinned: facts.HasSkinInstance || (facts.MeshVertexFlags & EffectVertexFlags.Skinned) != 0,
                Uv: (particle || meshHasUv) && source.Length > 0,
                MeshNormals: !particle && (facts.MeshVertexFlags & EffectVertexFlags.Normal) != 0,
                Subtextures: particle && facts.ParticleSubtextures > 0,
                Blend: blend,
                PaletteColor: paletteColor && greyscalePresent,
                PaletteAlpha: paletteAlpha && greyscalePresent,
                Falloff: falloff,
                Soft: soft,
                Lit: lit,
                EnvMapping: envMapping,
                ColorFalloff: colorFalloff,
                Decal: EffectShaderFlags.Has(facts.Flags1, EffectShaderFlags.Decal),
                ParticleNormalMask: normal.Length > 0 && envMask.Length > 0);
        }

        public static uint Descriptor(EffectFeatures f)
        {
            uint d = 0;
            if (f.VertexColors) d |= EffectDescriptorBits.VertexColors;
            if (f.Skinned) d |= EffectDescriptorBits.Skinned;
            if (f.Uv) d |= EffectDescriptorBits.Uv;
            if (f.Subtextures && f.Uv) d |= EffectDescriptorBits.Subtextures;
            bool falloff = f.Falloff && !f.Particle;
            if (falloff) d |= EffectDescriptorBits.Falloff;
            if (f.Blend == EffectBlend.Add) d |= EffectDescriptorBits.Additive;
            if (f.Blend == EffectBlend.Mult) d |= EffectDescriptorBits.Multiplicative;
            if (f.Particle) d |= EffectDescriptorBits.Particle;
            if (f.Lit) d |= EffectDescriptorBits.Lighting;
            if (f.Soft) d |= EffectDescriptorBits.SoftDepth;
            if (f.PaletteColor) d |= EffectDescriptorBits.PaletteColor;
            if (f.PaletteAlpha) d |= EffectDescriptorBits.PaletteAlpha;
            if (f.EnvMapping) d |= EffectDescriptorBits.EnvMapping;
            bool colorFalloff = f.ColorFalloff && !f.Particle;
            bool normals = f.MeshNormals && (falloff || colorFalloff || f.EnvMapping);
            if (normals) d |= EffectDescriptorBits.Normals;
            if (colorFalloff && normals) d |= EffectDescriptorBits.ColorFalloff;
            if (f.Particle && f.ParticleNormalMask) d |= EffectDescriptorBits.ParticleNormalMask;
            if (f.Decal) d |= EffectDescriptorBits.Decal;
            return d;
        }

        /// <summary>The vertex descriptor paired with a pixel descriptor, or null when the catalogue has none.</summary>
        public static uint? PairVertexDescriptor(uint pixelDescriptor, IEffectShaderCatalog catalog)
        {
            uint vertex = pixelDescriptor & ~EffectDescriptorBits.PixelOnly;
            if (catalog.HasVertexShader(vertex)) return vertex;
            vertex &= ~EffectDescriptorBits.VertexIrrelevant;
            return catalog.HasVertexShader(vertex) ? vertex : null;
        }

        /// <summary>Vertex attributes a shape's geometry provides for a vertex descriptor (semantic + index).</summary>
        public static IReadOnlySet<string> ProvidedAttributes(uint vertexDescriptor)
        {
            var attributes = new HashSet<string> { "POSITION0" };
            if ((vertexDescriptor & EffectDescriptorBits.Uv) != 0) attributes.Add("TEXCOORD0");
            if ((vertexDescriptor & EffectDescriptorBits.Particle) != 0) attributes.Add("TEXCOORD2");
            if ((vertexDescriptor & EffectDescriptorBits.Normals) != 0) attributes.Add("NORMAL0");
            if ((vertexDescriptor & EffectDescriptorBits.EnvMapping) != 0) attributes.Add("BINORMAL0");
            if ((vertexDescriptor & EffectDescriptorBits.VertexColors) != 0) attributes.Add("COLOR0");
            return attributes;
        }

        public static EffectSelection Select(EffectShapeFacts facts, IEffectShaderCatalog catalog)
        {
            var features = Features(facts);
            uint pixel = Descriptor(features);
            if (features.Skinned) return EffectSelection.Hidden(pixel, SkinnedReason);
            if (!catalog.HasPixelShader(pixel)) return EffectSelection.Hidden(pixel, NoPixelShaderReason);
            uint? vertex = PairVertexDescriptor(pixel, catalog);
            if (vertex is null) return EffectSelection.Hidden(pixel, NoVertexShaderReason);
            string? mismatch = catalog.CheckPair(vertex.Value, pixel, ProvidedAttributes(vertex.Value));
            return mismatch != null
                ? EffectSelection.Hidden(pixel, $"{SignatureMismatchReason}: {mismatch}")
                : new EffectSelection(vertex.Value, pixel, null);
        }
    }

    /// <summary>Effect family (shader archive group 4) read from the game's FXP: descriptors and the linking signatures.</summary>
    internal sealed class FxpEffectShaderCatalog : IEffectShaderCatalog
    {
        public const int EffectGroup = 4;

        private readonly Dictionary<uint, Lazy<ShaderInterface>> _vertex = new();
        private readonly Dictionary<uint, Lazy<ShaderInterface>> _pixel = new();

        private sealed record ShaderInterface(IReadOnlyList<SignatureElement> Inputs, IReadOnlyList<SignatureElement> Outputs);

        public FxpEffectShaderCatalog(FxpFile fxp)
        {
            if (EffectGroup >= fxp.Groups.Count)
                throw new InvalidDataException($"The shader archive has only {fxp.Groups.Count} groups; the effect family is group {EffectGroup}.");
            int vertexCount = (int)fxp.Groups[EffectGroup].VertexShaderCount;
            var entries = fxp.Entries.Where(e => e.GroupIndex == EffectGroup).OrderBy(e => e.BlobId).ToList();
            for (int i = 0; i < entries.Count; i++)
            {
                var entry = entries[i];
                var shader = new Lazy<ShaderInterface>(() => ReadInterface(entry));
                if (!(i < vertexCount ? _vertex : _pixel).TryAdd(entry.Descriptor, shader))
                    throw new InvalidDataException($"Effect descriptor 0x{entry.Descriptor:x8} appears twice in one stage.");
            }
        }

        /// <summary>Reads the effect family from the game's shader archive (<c>Data\Fallout4 - Shaders.ba2</c> under the install folder).</summary>
        public static FxpEffectShaderCatalog FromGameInstall(string fo4Location) =>
            new(ShaderArchiveReader.ReadFxp(AssetConverter.ShaderTranslation.Installation.ShaderTranslateStep.ShadersArchivePath(fo4Location)));

        public bool HasVertexShader(uint descriptor) => _vertex.ContainsKey(descriptor);

        public bool HasPixelShader(uint descriptor) => _pixel.ContainsKey(descriptor);

        public string? CheckPair(uint vertexDescriptor, uint pixelDescriptor, IReadOnlySet<string> providedAttributes)
        {
            var vertex = _vertex[vertexDescriptor].Value;
            var pixel = _pixel[pixelDescriptor].Value;
            foreach (var input in vertex.Inputs)
            {
                if (input.SystemValue != 0) continue;
                string attribute = $"{input.SemanticName}{input.SemanticIndex}";
                if (!providedAttributes.Contains(attribute))
                    return $"vertex input {attribute} is not provided";
            }
            foreach (var input in pixel.Inputs)
            {
                if (input.SystemValue != 0) continue;
                var output = vertex.Outputs.FirstOrDefault(o => o.Register == input.Register
                    && o.SemanticIndex == input.SemanticIndex
                    && string.Equals(o.SemanticName, input.SemanticName, StringComparison.OrdinalIgnoreCase));
                if (output == null)
                    return $"pixel input {input.SemanticName}{input.SemanticIndex} (register {input.Register}) is not written by the vertex shader";
                byte read = input.ReadWriteMask != 0 ? input.ReadWriteMask : input.Mask;
                if ((read & ~output.Mask) != 0)
                    return $"pixel input {input.SemanticName}{input.SemanticIndex} reads components the vertex shader does not write";
            }
            return null;
        }

        private static ShaderInterface ReadInterface(FxpEntry entry)
        {
            var container = DxbcContainer.Parse(entry.Dxbc);
            return new ShaderInterface(container.ReadSignature("ISGN", "ISG1"), container.ReadSignature("OSGN", "OSG1"));
        }
    }
}
