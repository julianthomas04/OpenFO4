using System.Reflection;
using AssetConverter;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Mutagen.Bethesda.Plugins.Records;
using Xunit;

namespace InstallerTests
{
    public sealed class WeaponModJsonTests : IDisposable
    {
        readonly SchemaDatabase _db = SchemaDatabase.CreateAsync().GetAwaiter().GetResult();

        public void Dispose() => _db.Dispose();

        [Fact]
        public async Task ResolveAttachPointNames_KeywordWithoutEditorId_FallsBackToTheFormKey()
        {
            _db.Execute("INSERT INTO keywords (form_key, editor_id) VALUES ('000001:A.esm', NULL), ('000002:A.esm', 'ap_gun_Grip')");
            _db.Execute("INSERT INTO object_modifications (form_key, attach_point, attach_parent_slots) VALUES ('000010:A.esm', '000001:A.esm', '[\"000002:A.esm\",\"000001:A.esm\",\"0000FF:A.esm\"]')");

            await WeaponModJson.ResolveAttachPointNamesAsync(_db.Connection, null);

            Assert.Equal("000001:A.esm", _db.QuerySingle<string>("SELECT attach_point_name FROM object_modifications"));
            Assert.Equal("[\"ap_gun_Grip\",\"000001:A.esm\",\"0000FF:A.esm\"]", _db.QuerySingle<string>("SELECT attach_parent_slots FROM object_modifications"));
        }

        [Fact]
        public async Task ResolveAttachPointNames_ModWithoutAttachPoint_StaysNull()
        {
            _db.Execute("INSERT INTO object_modifications (form_key) VALUES ('000010:A.esm')");

            await WeaponModJson.ResolveAttachPointNamesAsync(_db.Connection, null);

            Assert.Null(_db.QuerySingle<string?>("SELECT attach_point_name FROM object_modifications"));
            Assert.Null(_db.QuerySingle<string?>("SELECT attach_parent_slots FROM object_modifications"));
        }

        [Fact]
        public async Task ResolveAttachPointNames_ManySlots_KeepTheirStoredOrder()
        {
            var slotKeys = Enumerable.Range(0, 40).Select(i => $"{(i * 7919) % 1000 + 1:X6}:A.esm").Distinct().ToList();
            _db.Execute($"INSERT INTO keywords (form_key, editor_id) VALUES {string.Join(", ", slotKeys.Select((key, i) => $"('{key}', 'slot_{i}')"))}");
            _db.Execute("INSERT INTO object_modifications (form_key, attach_parent_slots) VALUES ('000010:A.esm', @slots)",
                new { slots = System.Text.Json.JsonSerializer.Serialize(slotKeys) });

            await WeaponModJson.ResolveAttachPointNamesAsync(_db.Connection, null);

            var expected = slotKeys.Select((_, i) => $"slot_{i}");
            Assert.Equal(expected, System.Text.Json.JsonSerializer.Deserialize<string[]>(_db.QuerySingle<string>("SELECT attach_parent_slots FROM object_modifications")));
        }

        static FormKey Key(uint id) => new(ModKey.FromNameAndExtension("A.esm"), id);

        static ObjectTemplate<Weapon.Property> DefaultTemplate(string name, params uint[] modIds) => new()
        {
            Name = name,
            Default = true,
            Includes = [.. modIds.Select(id => new ObjectTemplateInclude { Mod = new FormLink<IObjectModificationGetter>(Key(id)) })],
        };

        [Fact]
        public void DefaultMods_WithSeveralDefaultCombinations_UseTheFirstOne()
        {
            var weapon = new Weapon(Key(1), Fallout4Release.Fallout4)
            {
                ObjectTemplates = [DefaultTemplate("First", 0x10, 0x11), DefaultTemplate("Second", 0x20)],
            };

            Assert.Equal($"[\"{Key(0x10)}\",\"{Key(0x11)}\"]", WeaponModJson.DefaultMods(weapon));
        }

        [Fact]
        public void ModProperties_OfAnUnknownPropertySubtype_AreRejectedLoudly()
        {
            var mod = StubGetter.Create<IWeaponModificationGetter>(nameof(IWeaponModificationGetter.Properties),
                new List<IAObjectModPropertyGetter<Weapon.Property>> { StubGetter.Create<IAObjectModPropertyGetter<Weapon.Property>>() });

            Assert.Throws<NotSupportedException>(() => WeaponModJson.ModProperties(mod));
        }

        [Fact]
        public async Task Mod_WithoutWeaponArmorOrNpcTarget_HasNullTargetAndPropertiesAndIsSummarised()
        {
            var processor = new ObjectModificationProcessor();
            var records = new IAObjectModificationGetter[] { new ObjectModification(Key(0x30), Fallout4Release.Fallout4), new ObjectModification(Key(0x31), Fallout4Release.Fallout4), new WeaponModification(Key(0x32), Fallout4Release.Fallout4) };

            await processor.ProcessRecordsAsync(records, "OMOD", _db.Connection, silent: true, ct: TestContext.Current.CancellationToken);

            Assert.Equal(2, _db.QuerySingle<long>("SELECT COUNT(*) FROM object_modifications WHERE target_type IS NULL AND properties IS NULL"));
            Assert.Equal("WEAP", _db.QuerySingle<string>("SELECT target_type FROM object_modifications WHERE form_key = @key", new { key = Key(0x32).ToString() }));
            Assert.Equal(["  WARNING: 2 ObjectModification object modification(s) have no weapon, armor or NPC target; target_type stored as NULL."], processor.UntargetedSummaryLines());
        }

        // Hands back the configured members and a default value for every other one: the Mutagen getters are far too wide to implement by hand.
        // DispatchProxy requires an unsealed base type.
#pragma warning disable CA1852
        class StubGetter : DispatchProxy
        {
            Dictionary<string, object?> _members = new();

            public static T Create<T>(string? member = null, object? value = null) where T : class
            {
                var proxy = Create<T, StubGetter>();
                if (member != null) ((StubGetter)(object)proxy)._members[member] = value;
                return proxy;
            }

            protected override object? Invoke(MethodInfo? method, object?[]? args)
            {
                string name = method!.Name.StartsWith("get_", StringComparison.Ordinal) ? method.Name[4..] : method.Name;
                if (_members.TryGetValue(name, out var value)) return value;
                return method.ReturnType.IsValueType && method.ReturnType != typeof(void) ? System.Activator.CreateInstance(method.ReturnType) : null;
            }
        }
#pragma warning restore CA1852
    }
}
