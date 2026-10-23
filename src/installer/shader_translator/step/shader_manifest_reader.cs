using System.Text.Json;
using System.Text.RegularExpressions;
using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Wgsl;

namespace AssetConverter.ShaderTranslation.Installation
{
    /// <summary>Strict reader of the checked-in <c>shader_manifest.json</c>: any unknown field, duplicate or malformed value is a typed error.</summary>
    public static partial class ShaderManifestReader
    {
        public const string RepoRelativePath = "src/installer/shader_translator/shader_manifest.json";

        [GeneratedRegex(@"^[A-Za-z_][A-Za-z0-9_]*$")]
        private static partial Regex IdentifierPattern();

        [GeneratedRegex(@"^[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*)*$")]
        private static partial Regex MemberPathPattern();

        [GeneratedRegex(@"^(cb|t|s|u)(0|[1-9][0-9]{0,8})$")]
        private static partial Regex RegisterPattern();

        [GeneratedRegex(@"^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$", RegexOptions.IgnoreCase)]
        private static partial Regex WindowsReservedFileNamePattern();

        private static readonly string[] InterfaceStructSuffixes = ["_in", "_out"];

        public static ShaderManifest ReadFile(string path)
        {
            string text;
            try
            {
                text = File.ReadAllText(path);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                throw new ShaderManifestException($"Cannot read shader manifest {path}: {ex.Message}", ex);
            }
            return Parse(text, path);
        }

        public static ShaderManifest Parse(string json, string source = "shader manifest")
        {
            JsonDocument document;
            try
            {
                document = JsonDocument.Parse(json, new JsonDocumentOptions { CommentHandling = JsonCommentHandling.Disallow, AllowTrailingCommas = false });
            }
            catch (JsonException ex)
            {
                throw new ShaderManifestException($"{source}: invalid JSON: {ex.Message}", ex);
            }
            using (document)
            {
                var root = RequireObject(document.RootElement, source);
                CheckFields(root, ["shaders", "families"], source);
                var shaders = RequireArray(Require(root, "shaders", source), $"{source}: 'shaders'");
                var result = new List<ManifestShader>();
                int index = 0;
                foreach (var element in shaders.EnumerateArray())
                    result.Add(ParseShader(element, $"{source}: shaders[{index++}]"));
                CheckShaderNames(result, source);

                var families = new List<ManifestFamily>();
                if (root.TryGetProperty("families", out var familiesElement))
                {
                    index = 0;
                    foreach (var element in RequireArray(familiesElement, $"{source}: 'families'").EnumerateArray())
                        families.Add(ParseFamily(element, $"{source}: families[{index++}]"));
                    CheckFamilies(families, source);
                }
                return new ShaderManifest(result, families);
            }
        }

        /// <summary>
        /// Names become <c>&lt;name&gt;.wgsl</c> files (case-insensitive on Windows) and <c>&lt;name&gt;_in</c> / <c>&lt;name&gt;_out</c> structs,
        /// so two names must differ beyond case and neither may be the other plus an interface suffix.
        /// </summary>
        internal static void CheckShaderNames(IReadOnlyList<ManifestShader> shaders, string source)
        {
            var names = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (var shader in shaders)
            {
                if (!names.Add(shader.Name))
                    throw new ShaderManifestException($"{source}: duplicate shader name '{shader.Name}' (names are compared case-insensitively because they become file names).");
                if (WindowsReservedFileNamePattern().IsMatch(shader.Name))
                    throw new ShaderManifestException($"{source}: shader name '{shader.Name}' is a reserved Windows device name and cannot be a file name.");
            }
            foreach (var shader in shaders)
            {
                foreach (string suffix in InterfaceStructSuffixes)
                    if (names.Contains(shader.Name + suffix))
                        throw new ShaderManifestException($"{source}: shader name '{shader.Name}{suffix}' collides with the interface struct of shader '{shader.Name}'.");
            }
        }

        private static ManifestShader ParseShader(JsonElement element, string where)
        {
            var obj = RequireObject(element, where);
            CheckFields(obj, ["name", "blob", "stage", "nan_policy", "bindings"], where);
            string name = RequireString(Require(obj, "name", where), $"{where}.name");
            if (!IdentifierPattern().IsMatch(name) || name.StartsWith("__", StringComparison.Ordinal))
                throw new ShaderManifestException($"{where}: shader name '{name}' is not a valid WGSL identifier.");
            where = $"{where} ('{name}')";
            int blob = RequireNonNegativeInt(Require(obj, "blob", where), $"{where}.blob");
            var stage = ParseStage(RequireString(Require(obj, "stage", where), $"{where}.stage"), where);
            var nan = ParseNanPolicy(obj, where);
            var bindings = obj.TryGetProperty("bindings", out var bindingsElement)
                ? ParseBindings(bindingsElement, $"{where}.bindings")
                : [];
            CheckBindingUniqueness(bindings, where);
            return new ManifestShader(name, blob, stage, nan, bindings);
        }

        private static NanPolicy ParseNanPolicy(JsonElement obj, string where)
        {
            if (!obj.TryGetProperty("nan_policy", out var nanElement))
                return NanPolicy.Unspecified;
            return RequireString(nanElement, $"{where}.nan_policy") switch
            {
                "default" => NanPolicy.Unspecified,
                "d3d_strict" => NanPolicy.D3dStrict,
                var other => throw new ShaderManifestException($"{where}: unknown nan_policy '{other}' (expected default or d3d_strict)."),
            };
        }

        private static List<ManifestBinding> ParseBindings(JsonElement element, string where)
        {
            var bindings = new List<ManifestBinding>();
            int index = 0;
            foreach (var b in RequireArray(element, where).EnumerateArray())
                bindings.Add(ParseBinding(b, $"{where}[{index++}]"));
            return bindings;
        }

        private static ManifestFamily ParseFamily(JsonElement element, string where)
        {
            var obj = RequireObject(element, where);
            CheckFields(obj, ["family", "fxp_group", "name", "vertex_count", "pixel_count", "nan_policy", "bindings"], where);
            string family = RequireString(Require(obj, "family", where), $"{where}.family");
            if (!IdentifierPattern().IsMatch(family))
                throw new ShaderManifestException($"{where}: family '{family}' is not an identifier.");
            where = $"{where} ('{family}')";
            int group = RequireNonNegativeInt(Require(obj, "fxp_group", where), $"{where}.fxp_group");
            string template = RequireString(Require(obj, "name", where), $"{where}.name");
            CheckNameTemplate(template, where);
            int vertexCount = RequireNonNegativeInt(Require(obj, "vertex_count", where), $"{where}.vertex_count");
            int pixelCount = RequireNonNegativeInt(Require(obj, "pixel_count", where), $"{where}.pixel_count");
            var nan = ParseNanPolicy(obj, where);

            var bindingsObject = RequireObject(Require(obj, "bindings", where), $"{where}.bindings");
            CheckFields(bindingsObject, ["vertex", "fragment"], $"{where}.bindings");
            var vertexBindings = ParseBindings(Require(bindingsObject, "vertex", $"{where}.bindings"), $"{where}.bindings.vertex");
            var pixelBindings = ParseBindings(Require(bindingsObject, "fragment", $"{where}.bindings"), $"{where}.bindings.fragment");
            CheckBindingUniqueness(vertexBindings, $"{where}.bindings.vertex");
            CheckBindingUniqueness(pixelBindings, $"{where}.bindings.fragment");
            return new ManifestFamily(family, group, template, vertexCount, pixelCount, nan, vertexBindings, pixelBindings);
        }

        private static void CheckNameTemplate(string template, string where)
        {
            foreach (string placeholder in ManifestFamily.RequiredPlaceholders)
                if (!template.Contains(placeholder, StringComparison.Ordinal))
                    throw new ShaderManifestException($"{where}: name template '{template}' must contain {placeholder}.");
            string sample = ManifestFamily.ExpandName(template, "vs", 0);
            if (!IdentifierPattern().IsMatch(sample) || sample.StartsWith("__", StringComparison.Ordinal))
                throw new ShaderManifestException($"{where}: name template '{template}' does not expand to a valid WGSL identifier.");
        }

        private static void CheckFamilies(List<ManifestFamily> families, string source)
        {
            var groups = new HashSet<int>();
            var names = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (var family in families)
            {
                if (!groups.Add(family.FxpGroup))
                    throw new ShaderManifestException($"{source}: FXP group {family.FxpGroup} is claimed by two families.");
                if (!names.Add(family.Family))
                    throw new ShaderManifestException($"{source}: duplicate family '{family.Family}'.");
            }
        }

        private static ManifestBinding ParseBinding(JsonElement element, string where)
        {
            var obj = RequireObject(element, where);
            CheckFields(obj, ["register", "name", "group", "binding", "external", "texture_override", "uav_format"], where);
            string register = RequireString(Require(obj, "register", where), $"{where}.register");
            var match = RegisterPattern().Match(register);
            if (!match.Success)
                throw new ShaderManifestException($"{where}: bad register '{register}' (expected cbN, tN, sN or uN).");
            var kind = match.Groups[1].Value switch
            {
                "cb" => RegisterKind.ConstantBuffer,
                "t" => RegisterKind.Resource,
                "s" => RegisterKind.Sampler,
                _ => RegisterKind.UnorderedAccess,
            };
            int number = int.Parse(match.Groups[2].Value, System.Globalization.CultureInfo.InvariantCulture);
            where = $"{where} ({register})";

            bool external = obj.TryGetProperty("external", out var externalElement) && RequireBool(externalElement, $"{where}.external");
            string name = RequireString(Require(obj, "name", where), $"{where}.name");
            var namePattern = external ? MemberPathPattern() : IdentifierPattern();
            if (!namePattern.IsMatch(name))
                throw new ShaderManifestException(external
                    ? $"{where}: external name '{name}' must be an identifier or a dotted member path."
                    : $"{where}: name '{name}' must be a plain WGSL identifier (dotted paths are only allowed for external bindings).");

            var textureOverride = TextureOverride.None;
            if (obj.TryGetProperty("texture_override", out var overrideElement))
            {
                if (kind != RegisterKind.Resource)
                    throw new ShaderManifestException($"{where}: texture_override is only valid on t registers.");
                textureOverride = RequireString(overrideElement, $"{where}.texture_override") switch
                {
                    "depth" => TextureOverride.Depth,
                    "color" => TextureOverride.Color,
                    var other => throw new ShaderManifestException($"{where}: unknown texture_override '{other}' (expected depth or color)."),
                };
            }

            string? uavFormat = null;
            if (obj.TryGetProperty("uav_format", out var formatElement))
            {
                if (kind != RegisterKind.UnorderedAccess)
                    throw new ShaderManifestException($"{where}: uav_format is only valid on u registers.");
                uavFormat = RequireString(formatElement, $"{where}.uav_format");
                if (!WgslStorageFormats.IsKnown(uavFormat))
                    throw new ShaderManifestException($"{where}: uav_format '{uavFormat}' is not a supported WGSL storage format.");
            }

            return new ManifestBinding(
                kind, number, name,
                SlotNumber(obj, "group", external, where),
                SlotNumber(obj, "binding", external, where),
                external, textureOverride, uavFormat);
        }

        /// <summary>External bindings are declared by the wrapper, so their group/binding numbers carry no meaning and may be omitted.</summary>
        private static int SlotNumber(JsonElement obj, string field, bool external, string where) =>
            external && !obj.TryGetProperty(field, out _) ? 0 : RequireNonNegativeInt(Require(obj, field, where), $"{where}.{field}");

        private static void CheckBindingUniqueness(List<ManifestBinding> bindings, string where)
        {
            var registers = new HashSet<(RegisterKind, int)>();
            var slots = new HashSet<(int, int)>();
            var names = new HashSet<string>(StringComparer.Ordinal);
            foreach (var b in bindings)
            {
                if (!registers.Add((b.Kind, b.Register)))
                    throw new ShaderManifestException($"{where}: register {b.Kind}{b.Register} is bound twice.");
                if (b.External) continue;
                if (!slots.Add((b.Group, b.Binding)))
                    throw new ShaderManifestException($"{where}: @group({b.Group}) @binding({b.Binding}) is used by two declared bindings.");
                if (!names.Add(b.Name))
                    throw new ShaderManifestException($"{where}: binding name '{b.Name}' is used twice.");
            }
        }

        private static ShaderStage ParseStage(string stage, string where) => stage switch
        {
            "vertex" => ShaderStage.Vertex,
            "fragment" => ShaderStage.Pixel,
            "compute" => ShaderStage.Compute,
            _ => throw new ShaderManifestException($"{where}: unknown stage '{stage}' (expected vertex, fragment or compute)."),
        };

        private static JsonElement Require(JsonElement obj, string name, string where) =>
            obj.TryGetProperty(name, out var value) ? value : throw new ShaderManifestException($"{where}: missing field '{name}'.");

        private static JsonElement RequireObject(JsonElement element, string where) =>
            element.ValueKind == JsonValueKind.Object ? element : throw new ShaderManifestException($"{where}: expected an object.");

        private static JsonElement RequireArray(JsonElement element, string where) =>
            element.ValueKind == JsonValueKind.Array ? element : throw new ShaderManifestException($"{where}: expected an array.");

        private static string RequireString(JsonElement element, string where) =>
            element.ValueKind == JsonValueKind.String ? element.GetString()! : throw new ShaderManifestException($"{where}: expected a string.");

        private static bool RequireBool(JsonElement element, string where) => element.ValueKind switch
        {
            JsonValueKind.True => true,
            JsonValueKind.False => false,
            _ => throw new ShaderManifestException($"{where}: expected true or false."),
        };

        private static int RequireNonNegativeInt(JsonElement element, string where)
        {
            if (element.ValueKind != JsonValueKind.Number || !element.TryGetInt32(out int value) || value < 0)
                throw new ShaderManifestException($"{where}: expected a non-negative 32-bit integer.");
            return value;
        }

        private static void CheckFields(JsonElement obj, string[] allowed, string where)
        {
            var seen = new HashSet<string>(StringComparer.Ordinal);
            foreach (var property in obj.EnumerateObject())
            {
                if (!allowed.Contains(property.Name, StringComparer.Ordinal))
                    throw new ShaderManifestException($"{where}: unknown field '{property.Name}'.");
                if (!seen.Add(property.Name))
                    throw new ShaderManifestException($"{where}: duplicate field '{property.Name}'.");
            }
        }
    }
}
