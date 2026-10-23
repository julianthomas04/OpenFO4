using System.Drawing;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using AssetConverter;
using InstallerTests.Support;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Xunit;

namespace InstallerTests
{
    // The JSON is stored in the ESM database and parsed by the engine's RawLighting
    // (src/engine/src/rendering/lighting/cell_lighting.rs). Property names are the contract.
    public class LightingJsonTests
    {
        private const string EngineLightingSource = "src/engine/src/rendering/lighting/cell_lighting.rs";

        private static readonly JsonSerializerOptions StoredJson = new() { DefaultIgnoreCondition = System.Text.Json.Serialization.JsonIgnoreCondition.WhenWritingNull };

        [Fact]
        public void ProjectCell_NullLighting_IsNull()
        {
            Assert.Null(LightingJson.Project((ICellLightingGetter?)null));
        }

        [Fact]
        public void ProjectCell_Colours_AreRgbObjectsFromTheMatchingSourceFields()
        {
            var cell = new CellLighting
            {
                AmbientColor = Color.FromArgb(1, 2, 3),
                DirectionalColor = Color.FromArgb(4, 5, 6),
                FogNearColor = Color.FromArgb(7, 8, 9),
                FogFarColor = Color.FromArgb(10, 11, 12),
                ForColorHighNear = Color.FromArgb(13, 14, 15),
                ForColorHighFar = Color.FromArgb(16, 17, 18),
            };

            var json = Serialize(LightingJson.Project(cell));

            Assert.Equal(new[] { 1, 2, 3 }, Rgb(json, "AmbientColor"));
            Assert.Equal(new[] { 4, 5, 6 }, Rgb(json, "DirectionalColor"));
            Assert.Equal(new[] { 7, 8, 9 }, Rgb(json, "FogNearColor"));
            Assert.Equal(new[] { 10, 11, 12 }, Rgb(json, "FogFarColor"));
            Assert.Equal(new[] { 13, 14, 15 }, Rgb(json, "FogHighNearColor"));
            Assert.Equal(new[] { 16, 17, 18 }, Rgb(json, "FogHighFarColor"));
        }

        [Fact]
        public void ProjectCell_ScalarFields_AreCopiedUnderTheirOwnNames()
        {
            var cell = new CellLighting
            {
                FogNear = 1.5f,
                FogFar = 2.5f,
                DirectionalRotationXY = 30,
                DirectionalRotationZ = 40,
                DirectionalFade = 0.75f,
                FogClipDistance = 3.5f,
                FogPower = 4.5f,
                FogMax = 0.9f,
            };

            var json = Serialize(LightingJson.Project(cell));

            Assert.Equal(1.5f, json["FogNear"]!.GetValue<float>());
            Assert.Equal(2.5f, json["FogFar"]!.GetValue<float>());
            Assert.Equal(30, json["DirectionalRotationXY"]!.GetValue<int>());
            Assert.Equal(40, json["DirectionalRotationZ"]!.GetValue<int>());
            Assert.Equal(0.75f, json["DirectionalFade"]!.GetValue<float>());
            Assert.Equal(3.5f, json["FogClipDistance"]!.GetValue<float>());
            Assert.Equal(4.5f, json["FogPower"]!.GetValue<float>());
            Assert.Equal(0.9f, json["FogMax"]!.GetValue<float>());
        }

        [Fact]
        public void ProjectCell_InheritsFlags_AreWrittenAsTheRawBitfield()
        {
            var cell = new CellLighting
            {
                Inherits = CellLighting.Inherit.AmbientColor | CellLighting.Inherit.FogNear | CellLighting.Inherit.LightFadeDistances,
            };

            var json = Serialize(LightingJson.Project(cell));

            // Engine Inherits layout: AMBIENT_COLOR = 1 << 0, FOG_NEAR = 1 << 3, LIGHT_FADE_DISTANCES = 1 << 10.
            Assert.Equal((1u << 0) | (1u << 3) | (1u << 10), json["Inherits"]!.GetValue<uint>());
        }

        [Fact]
        public void ProjectCell_AmbientCube_WritesAllSixFacesSpecularAndScale()
        {
            var cell = new CellLighting();
            cell.AmbientColors.DirectionalXPlus = Color.FromArgb(1, 1, 1);
            cell.AmbientColors.DirectionalXMinus = Color.FromArgb(2, 2, 2);
            cell.AmbientColors.DirectionalYPlus = Color.FromArgb(3, 3, 3);
            cell.AmbientColors.DirectionalYMinus = Color.FromArgb(4, 4, 4);
            cell.AmbientColors.DirectionalZPlus = Color.FromArgb(5, 5, 5);
            cell.AmbientColors.DirectionalZMinus = Color.FromArgb(6, 6, 6);
            cell.AmbientColors.Specular = Color.FromArgb(7, 7, 7);
            cell.AmbientColors.Scale = 2.5f;

            var cube = Serialize(LightingJson.Project(cell))["AmbientColors"]!;

            Assert.Equal(1, cube["DirectionalXPlus"]!["R"]!.GetValue<int>());
            Assert.Equal(2, cube["DirectionalXMinus"]!["R"]!.GetValue<int>());
            Assert.Equal(3, cube["DirectionalYPlus"]!["R"]!.GetValue<int>());
            Assert.Equal(4, cube["DirectionalYMinus"]!["R"]!.GetValue<int>());
            Assert.Equal(5, cube["DirectionalZPlus"]!["R"]!.GetValue<int>());
            Assert.Equal(6, cube["DirectionalZMinus"]!["R"]!.GetValue<int>());
            Assert.Equal(7, cube["Specular"]!["R"]!.GetValue<int>());
            Assert.Equal(2.5f, cube["Scale"]!.GetValue<float>());
        }

        [Fact]
        public void ProjectTemplate_RenamedFields_UseTemplateSpecificNames()
        {
            var template = NewTemplate();
            template.DirectionalAmbientColors!.DirectionalZPlus = Color.FromArgb(9, 9, 9);
            template.FogColorHighNear = Color.FromArgb(20, 21, 22);
            template.FogColorHighFar = Color.FromArgb(23, 24, 25);
            template.LightFadeStartDistance = 11f;
            template.LightFadeEndDistance = 12f;

            var json = Serialize(LightingJson.Project(template));

            Assert.Equal(9, json["DirectionalAmbientColors"]!["DirectionalZPlus"]!["R"]!.GetValue<int>());
            Assert.Equal(new[] { 20, 21, 22 }, Rgb(json, "FogColorHighNear"));
            Assert.Equal(new[] { 23, 24, 25 }, Rgb(json, "FogColorHighFar"));
            Assert.Equal(11f, json["LightFadeStartDistance"]!.GetValue<float>());
            Assert.Equal(12f, json["LightFadeEndDistance"]!.GetValue<float>());
        }

        [Fact]
        public void ProjectCell_FieldsTheEngineReads_AreAllPresent()
        {
            var json = Serialize(LightingJson.Project(new CellLighting()));

            Assert.All(EngineRawLightingProperties(), field =>
                Assert.True(json.ContainsKey(field.Name) || field.Aliases.Any(json.ContainsKey), $"cell JSON lacks engine field {field.Name}"));
        }

        [Fact]
        public void ProjectTemplate_FieldsTheEngineReads_AreAllPresentExceptInherits()
        {
            var json = Serialize(LightingJson.Project(NewTemplate()));

            var expected = EngineRawLightingProperties().Where(field => field.Name != "Inherits");
            Assert.All(expected, field =>
                Assert.True(json.ContainsKey(field.Name) || field.Aliases.Any(json.ContainsKey), $"template JSON lacks engine field {field.Name}"));
        }

        [Fact]
        public void ProjectCell_AmbientCubeFieldsTheEngineReads_AreAllPresent()
        {
            string cubeBody = RepoFiles.ExtractBlock(EngineLightingSource, @"struct RawAmbientCube \{(.*?)\}", "struct RawAmbientCube");
            var cubeFields = Regex.Matches(cubeBody, @"(\w+):\s*RawColor").Select(m => PascalCase(m.Groups[1].Value)).ToList();
            Assert.True(cubeFields.Count == 6, $"parsed {cubeFields.Count} ambient cube faces from {EngineLightingSource}, expected 6");

            var cube = Serialize(LightingJson.Project(new CellLighting()))["AmbientColors"]!.AsObject();

            Assert.All(cubeFields, field => Assert.True(cube.ContainsKey(field), $"ambient cube JSON lacks engine field {field}"));
        }

        [Fact]
        public void ProjectCell_ColourObjects_HaveTheRgbKeysTheEngineReads()
        {
            string colorBody = RepoFiles.ExtractBlock(EngineLightingSource, @"struct RawColor \{(.*?)\}", "struct RawColor");
            var colorFields = Regex.Matches(colorBody, @"(\w+):\s*u8").Select(m => PascalCase(m.Groups[1].Value)).ToList();

            var ambient = Serialize(LightingJson.Project(new CellLighting()))["AmbientColor"]!.AsObject();

            Assert.Equal(new[] { "R", "G", "B" }, colorFields);
            Assert.Equal(colorFields.Order(), ambient.Select(p => p.Key).Order());
        }

        private static LightingTemplate NewTemplate() => new(FormKey.Factory("000800:Test.esp"), Fallout4Release.Fallout4)
        {
            DirectionalAmbientColors = new AmbientColors(),
        };

        private static JsonObject Serialize(object? projection) =>
            JsonNode.Parse(JsonSerializer.Serialize(projection, StoredJson))!.AsObject();

        private static int[] Rgb(JsonObject json, string property) =>
            new[] { json[property]!["R"]!.GetValue<int>(), json[property]!["G"]!.GetValue<int>(), json[property]!["B"]!.GetValue<int>() };

        private sealed record EngineField(string Name, string[] Aliases);

        // Reads the serde field list of RawLighting: PascalCase of the Rust name unless renamed, plus aliases.
        private static List<EngineField> EngineRawLightingProperties()
        {
            string body = RepoFiles.ExtractBlock(EngineLightingSource, @"pub struct RawLighting \{(.*?)\n\}", "struct RawLighting");
            var fields = new List<EngineField>();
            string? rename = null;
            var aliases = new List<string>();
            foreach (string line in body.Split('\n').Select(l => l.Trim()))
            {
                var renameMatch = Regex.Match(line, "rename = \"(\\w+)\"");
                var aliasMatch = Regex.Match(line, "alias = \"(\\w+)\"");
                var fieldMatch = Regex.Match(line, @"^(\w+):\s*Option<");
                if (renameMatch.Success) rename = renameMatch.Groups[1].Value;
                if (aliasMatch.Success) aliases.Add(aliasMatch.Groups[1].Value);
                if (!fieldMatch.Success) continue;
                fields.Add(new EngineField(rename ?? PascalCase(fieldMatch.Groups[1].Value), aliases.ToArray()));
                rename = null;
                aliases.Clear();
            }

            Assert.True(fields.Count >= 10, $"parsed only {fields.Count} RawLighting fields from {EngineLightingSource}; the field syntax changed, update the test parser");
            return fields;
        }

        private static string PascalCase(string snake) =>
            string.Concat(snake.Split('_').Select(part => char.ToUpperInvariant(part[0]) + part[1..]));
    }
}
