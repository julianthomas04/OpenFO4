using System.Text.Json;
using Xunit;

namespace InstallerTests
{
    // Weapon assembly data:
    // default combination, object templates, OMOD model/attach data, keyword form-key lists.
    [Collection(FixtureEsm.Name)]
    public class EsmLoadWeaponTests(LoadedFixtureDatabase db)
    {
        string? WeaponColumn(string column, string editorId) =>
            db.QuerySingle<string?>($"SELECT {column} FROM weapons WHERE form_key = @key", new { key = db.KeyOf(editorId) });

        sealed class ModRow
        {
            public string? TargetType { get; init; }
            public string? AttachPoint { get; init; }
            public string? AttachPointName { get; init; }
            public string? AttachParentSlots { get; init; }
            public string? ModelPath { get; init; }
            public string? Includes { get; init; }
            public string? Properties { get; init; }
        }

        ModRow Mod(string editorId) =>
            db.QuerySingle<ModRow>("SELECT * FROM object_modifications WHERE form_key = @key", new { key = db.KeyOf(editorId) });

        static string[] Strings(string? json) => JsonSerializer.Deserialize<string[]>(json!)!;

        [Fact]
        public void DefaultMods_AreTheIncludesOfTheDefaultCombinationInOrder()
        {
            Assert.Equal([db.KeyOf("ModGrip"), db.KeyOf("ModNullMuzzle")], Strings(WeaponColumn("default_mods", "TestPistol")));
        }

        [Fact]
        public void DefaultMods_WithoutDefaultCombination_AreNull()
        {
            Assert.Null(WeaponColumn("default_mods", "TestPistolNoDefault"));
        }

        [Fact]
        public void WeaponWithoutObjectTemplate_HasNullTemplateColumns()
        {
            Assert.Null(WeaponColumn("default_mods", "TestGrenade"));
            Assert.Null(WeaponColumn("object_template", "TestGrenade"));
        }

        [Fact]
        public void ObjectTemplate_ListsEveryCombinationInRecordOrder()
        {
            using var document = JsonDocument.Parse(WeaponColumn("object_template", "TestPistol")!);
            var combinations = document.RootElement.EnumerateArray().ToList();

            Assert.Equal(2, combinations.Count);
            Assert.Equal(0, combinations[0].GetProperty("index").GetInt32());
            Assert.False(combinations[0].GetProperty("default").GetBoolean());
            Assert.Equal(1, combinations[1].GetProperty("index").GetInt32());
            Assert.True(combinations[1].GetProperty("default").GetBoolean());
            Assert.Equal("Default", combinations[1].GetProperty("name").GetString());
            Assert.Equal(-1, combinations[1].GetProperty("addon_index").GetInt32());

            var includes = combinations[1].GetProperty("includes").EnumerateArray().ToList();
            Assert.Equal(db.KeyOf("ModGrip"), includes[0].GetProperty("mod").GetString());
            Assert.True(includes[0].GetProperty("dont_use_all").GetBoolean());
            Assert.False(includes[1].GetProperty("dont_use_all").GetBoolean());
        }

        [Fact]
        public void Mod_WithModel_StoresModelAttachPointAndSlotsAsEditorIds()
        {
            var row = Mod("ModGrip");

            Assert.Equal("WEAP", row.TargetType);
            Assert.Equal(LoadedFixtureDatabase.GripModel, row.ModelPath);
            Assert.Equal(db.KeyOf(LoadedFixtureDatabase.GripAttachPoint), row.AttachPoint);
            Assert.Equal(LoadedFixtureDatabase.GripAttachPoint, row.AttachPointName);
            Assert.Equal([LoadedFixtureDatabase.MuzzleAttachPoint, LoadedFixtureDatabase.GripAttachPoint], Strings(row.AttachParentSlots));
        }

        [Fact]
        public void Mod_WithoutModel_HasNullModelPathButKeepsItsAttachPointName()
        {
            var row = Mod("ModNullMuzzle");

            Assert.Null(row.ModelPath);
            Assert.Equal(LoadedFixtureDatabase.MuzzleAttachPoint, row.AttachPointName);
            Assert.Equal("[]", row.AttachParentSlots);
        }

        [Fact]
        public void Mod_Includes_AreStoredWithTheirFlags()
        {
            using var document = JsonDocument.Parse(Mod("ModGrip").Includes!);
            var include = document.RootElement.EnumerateArray().Single();

            Assert.Equal(db.KeyOf("ModNullMuzzle"), include.GetProperty("mod").GetString());
            Assert.Equal(2, include.GetProperty("min_level").GetInt32());
            Assert.True(include.GetProperty("optional").GetBoolean());
            Assert.False(include.GetProperty("dont_use_all").GetBoolean());
        }

        [Fact]
        public void Mod_Properties_RecordTypeNameFunctionAndValues()
        {
            using var document = JsonDocument.Parse(Mod("ModGrip").Properties!);
            var properties = document.RootElement.EnumerateArray().ToList();

            Assert.Equal("Int", properties[0].GetProperty("value_type").GetString());
            Assert.Equal("Speed", properties[0].GetProperty("property").GetString());
            Assert.Equal([5, 6], properties[0].GetProperty("values").EnumerateArray().Select(value => value.GetInt32()));

            Assert.Equal("FormLinkInt", properties[1].GetProperty("value_type").GetString());
            var values = properties[1].GetProperty("values").EnumerateArray().ToList();
            Assert.Equal(db.KeyOf(LoadedFixtureDatabase.GripAttachPoint), values[0].GetString());
            Assert.Equal(3, values[1].GetInt32());
        }

        [Fact]
        public void ItemKeywords_AreFormKeyStringsInOrder()
        {
            var keywords = db.QuerySingle<string>("SELECT keywords FROM items WHERE form_key = @key", new { key = db.KeyOf("TestPistol") });

            Assert.Equal([db.KeyOf(LoadedFixtureDatabase.MuzzleAttachPoint), db.KeyOf(LoadedFixtureDatabase.GripAttachPoint)], Strings(keywords));
        }

        [Fact]
        public void ItemKeywords_WithoutKeywords_AreNull()
        {
            var keywords = db.QuerySingle<string?>("SELECT keywords FROM items WHERE form_key = @key", new { key = db.KeyOf("TestGrenade") });

            Assert.Null(keywords);
        }
    }
}
