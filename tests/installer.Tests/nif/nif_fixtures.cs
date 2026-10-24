using System.Numerics;
using NiflySharp;
using NiflySharp.Blocks;
using NiflySharp.Enums;
using NiflySharp.Structs;

namespace InstallerTests.Nif
{
    /// <summary>Builds small Fallout 4 NIF scenes in memory with NiflySharp itself, then saves them for the converter.</summary>
    internal sealed class NifSceneBuilder
    {
        public NifFile Nif { get; } = new();

        public NiNode Root { get; }

        public NifSceneBuilder()
        {
            Nif.Create(NiVersion.GetFO4(), true);
            Root = Nif.GetRootNode();
            Root.Name = new NiStringRef("Scene");
            SetTransform(Root, Vector3.Zero, Identity(), 1f);
        }

        public NiNode AddNode(NiNode parent, string name, Vector3 translation, Matrix33? rotation = null, float scale = 1f)
        {
            var node = new NiNode { Name = new NiStringRef(name) };
            SetTransform(node, translation, rotation ?? Identity(), scale);
            Attach(parent, node);
            return node;
        }

        public BSTriShape AddShape(NiNode parent, string name, Vector3[] positions, Triangle[] triangles, TexCoord[]? uvs = null,
            Vector3[]? normals = null, Vector3? translation = null, Matrix33? rotation = null, float scale = 1f)
        {
            var shape = CreateShape(name, positions, triangles, uvs, normals, translation, rotation, scale);
            Attach(parent, shape);
            return shape;
        }

        /// <summary>A shape that no node lists as a child, as the topmost block of its own hierarchy.</summary>
        public BSTriShape AddRootShape(string name, Vector3[] positions, Triangle[] triangles, Vector3? translation = null, Matrix33? rotation = null,
            float scale = 1f)
        {
            var shape = CreateShape(name, positions, triangles, null, null, translation, rotation, scale);
            Nif.AddBlock(shape);
            return shape;
        }

        private static BSTriShape CreateShape(string name, Vector3[] positions, Triangle[] triangles, TexCoord[]? uvs, Vector3[]? normals,
            Vector3? translation, Matrix33? rotation, float scale)
        {
            var shape = new BSTriShape { Name = new NiStringRef(name) };
            shape.Create(NiVersion.GetFO4(), positions.ToList(), triangles.ToList(),
                (uvs ?? positions.Select(_ => new TexCoord(0f, 0f)).ToArray()).ToList(),
                (normals ?? positions.Select(_ => Vector3.UnitZ).ToArray()).ToList());
            SetTransform(shape, translation ?? Vector3.Zero, rotation ?? Identity(), scale);
            return shape;
        }

        /// <summary>A BSWaterShaderProperty; NiflySharp keeps its fields private, so they are set by reflection.</summary>
        public static BSWaterShaderProperty CreateWaterShader(uint waterFlags, uint shaderFlags1, uint shaderFlags2, TexCoord uvOffset, TexCoord uvScale)
        {
            var shader = new BSWaterShaderProperty();
            SetField(shader, "_waterShaderFlags", waterFlags);
            SetField(shader, "_shaderFlags1", shaderFlags1);
            SetField(shader, "_shaderFlags2_SSPF2", shaderFlags2);
            SetField(shader, "_uVOffset", uvOffset);
            SetField(shader, "_uVScale", uvScale);
            return shader;
        }

        /// <summary>A BSEffectShaderProperty with the given inline fields; NiflySharp keeps them private, so they are set by reflection.</summary>
        public static BSEffectShaderProperty CreateEffectShader(
            Fallout4ShaderPropertyFlags1 flags1, Fallout4ShaderPropertyFlags2 flags2, string sourceTexture = "",
            float baseColorScale = 1f, Color4? baseColor = null, byte lightingInfluence = 255,
            (float StartAngle, float StopAngle, float StartOpacity, float StopOpacity)? falloff = null,
            string envMapTexture = "", string envMaskTexture = "", float envMapScale = 1f)
        {
            var shader = new BSEffectShaderProperty();
            SetField(shader, "_shaderFlags1_F4SPF1", flags1);
            SetField(shader, "_shaderFlags2_F4SPF2", flags2);
            SetField(shader, "_sourceTexture", NiString(sourceTexture));
            SetField(shader, "_envMapTexture", NiString(envMapTexture));
            SetField(shader, "_envMaskTexture", NiString(envMaskTexture));
            SetField(shader, "_baseColor", baseColor ?? new Color4(1f, 1f, 1f, 1f));
            SetField(shader, "_baseColorScale", baseColorScale);
            SetField(shader, "_lightingInfluence", lightingInfluence);
            SetField(shader, "_environmentMapScale_fl", envMapScale);
            var (start, stop, startOpacity, stopOpacity) = falloff ?? (1f, 0f, 1f, 0f);
            SetField(shader, "_falloffStartAngle", start);
            SetField(shader, "_falloffStopAngle", stop);
            SetField(shader, "_falloffStartOpacity", startOpacity);
            SetField(shader, "_falloffStopOpacity", stopOpacity);
            return shader;
        }

        public void AttachAlphaProperty(BSTriShape shape, bool blend, AlphaFunction source, AlphaFunction destination)
        {
            var property = new NiAlphaProperty();
            property.Flags.AlphaBlend = blend;
            property.Flags.SourceBlendMode = source;
            property.Flags.DestinationBlendMode = destination;
            shape.AlphaPropertyRef = new NiBlockRef<NiAlphaProperty>(Nif.AddBlock(property));
        }

        private static object NiString(string value)
        {
            var field = typeof(BSEffectShaderProperty).GetField("_sourceTexture", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance)!;
            object instance = Activator.CreateInstance(field.FieldType)!;
            field.FieldType.GetProperty("Content")!.SetValue(instance, value);
            return instance;
        }

        private static void SetField(BSEffectShaderProperty shader, string name, object value)
        {
            var field = typeof(BSEffectShaderProperty).GetField(name, System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance)
                ?? throw new InvalidOperationException($"NiflySharp has no field {name}");
            field.SetValue(shader, value);
        }

        public void AttachShader(BSTriShape shape, BSShaderProperty shader) =>
            shape.ShaderPropertyRef = new NiBlockRef<BSShaderProperty>(Nif.AddBlock(shader));

        private static void SetField(BSWaterShaderProperty shader, string name, object value)
        {
            var field = typeof(BSWaterShaderProperty).GetField(name, System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance)
                ?? throw new InvalidOperationException($"NiflySharp has no field {name}");
            field.SetValue(shader, value is uint raw && field.FieldType.IsEnum ? Enum.ToObject(field.FieldType, raw) : value);
        }

        public string Save(string directory, string fileName)
        {
            string path = Path.Combine(directory, fileName);
            int result = Nif.Save(path);
            if (result != 0)
                throw new InvalidOperationException($"NiflySharp could not save the fixture ({result})");
            return path;
        }

        public static Matrix33 Identity() => new() { M11 = 1f, M22 = 1f, M33 = 1f };

        /// <summary>Rotation by <paramref name="degrees"/> about +Z (v' = R v). NiflySharp names the properties M{column}{row}: the file's first three floats are M11, M21, M31.</summary>
        public static Matrix33 AboutZ(float degrees)
        {
            float c = MathF.Cos(degrees * MathF.PI / 180f);
            float s = MathF.Sin(degrees * MathF.PI / 180f);
            return new Matrix33 { M11 = c, M21 = -s, M12 = s, M22 = c, M33 = 1f };
        }

        /// <summary>Rotation by 180 degrees about +Y: the wrapper Bethesda tools put on root nodes.</summary>
        public static Matrix33 HalfTurnAboutY() => new() { M11 = -1f, M22 = 1f, M33 = -1f };

        public int Attach(NiNode parent, NiAVObject child)
        {
            int index = Nif.AddBlock(child);
            parent.Children ??= new NiBlockRefArray<NiAVObject>();
            parent.Children.AddBlockRef(index);
            return index;
        }

        private static void SetTransform(NiAVObject node, Vector3 translation, Matrix33 rotation, float scale)
        {
            node.Translation = translation;
            node.Rotation = rotation;
            node.Scale = scale;
        }
    }
}
