using System.Text.Json.Nodes;
using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Mutagen.Bethesda.Plugins.Records;

namespace AssetConverter
{
    // JSON shapes of weapon assembly data (OMOD rows, WEAP object templates) and the keyword editor-id post-pass that cannot run inside the concurrent record jobs.
    internal static class WeaponModJson
    {
        public static string? ModIncludes(IEnumerable<IObjectModIncludeGetter>? includes) =>
            includes == null ? null : new JsonArray(includes.Select(include => (JsonNode?)IncludeNode(
                include.Mod.FormKey, include.MinimumLevel, include.Optional, include.DoNotUseAll)).ToArray()).ToJsonString();

        static JsonObject IncludeNode(FormKey mod, byte minimumLevel, bool optional, bool doNotUseAll) => new JsonObject
        {
            ["mod"] = mod.ToString(),
            ["min_level"] = minimumLevel,
            ["optional"] = optional,
            ["dont_use_all"] = doNotUseAll,
        };

        static JsonArray TemplateIncludes(IEnumerable<IObjectTemplateIncludeGetter> includes) =>
            new(includes.Select(include => (JsonNode?)new JsonObject
            {
                ["mod"] = include.Mod.FormKey.ToString(),
                ["attach_point_index"] = include.AttachPointIndex,
                ["optional"] = include.Optional,
                ["dont_use_all"] = include.DontUseAll,
            }).ToArray());

        public static string? DefaultMods(IWeaponGetter weapon)
        {
            var template = weapon.ObjectTemplates?.FirstOrDefault(candidate => candidate.Default);
            if (template == null) return null;
            return StringArray(template.Includes.Select(include => include.Mod.FormKey.ToString())).ToJsonString();
        }

        public static string? ObjectTemplates(IWeaponGetter weapon)
        {
            if (weapon.ObjectTemplates == null) return null;
            var combinations = weapon.ObjectTemplates.Select((template, index) => (JsonNode?)new JsonObject
            {
                ["index"] = index,
                ["addon_index"] = template.AddonIndex,
                ["name"] = template.Name?.ToString(),
                ["default"] = template.Default,
                ["level_min"] = template.LevelMin,
                ["level_max"] = template.LevelMax,
                ["min_level_for_ranks"] = template.MinLevelForRanks,
                ["alt_levels_per_tier"] = template.AltLevelsPerTier,
                ["editor_only"] = template.IsEditorOnly,
                ["keywords"] = StringArray(template.Keywords?.Select(link => link.FormKey.ToString()) ?? []),
                ["includes"] = TemplateIncludes(template.Includes ?? []),
                ["properties"] = Properties(template.Properties),
            });
            return new JsonArray(combinations.ToArray()).ToJsonString();
        }

        static JsonArray StringArray(IEnumerable<string> values) =>
            new(values.Select(value => (JsonNode?)JsonValue.Create(value)).ToArray());

        static JsonArray Properties<TProperty>(IEnumerable<IAObjectModPropertyGetter<TProperty>>? properties)
            where TProperty : struct, Enum =>
            new((properties ?? []).Select(property => (JsonNode?)Property(property)).ToArray());

        static JsonObject Property<TProperty>(IAObjectModPropertyGetter<TProperty> property)
            where TProperty : struct, Enum
        {
            var (valueType, function, values) = property switch
            {
                IObjectModIntPropertyGetter<TProperty> p => ("Int", p.FunctionType.ToString(), Values(p.Value, p.Value2)),
                IObjectModFloatPropertyGetter<TProperty> p => ("Float", p.FunctionType.ToString(), Values(p.Value, p.Value2)),
                IObjectModBoolPropertyGetter<TProperty> p => ("Bool", p.FunctionType.ToString(), Values(p.Value, p.Value2)),
                IObjectModStringPropertyGetter<TProperty> p => ("String", p.FunctionType.ToString(), Values(p.Value)),
                IObjectModEnumPropertyGetter<TProperty> p => ("Enum", p.FunctionType.ToString(), Values(p.EnumIntValue)),
                IObjectModFormLinkIntPropertyGetter<TProperty> p => ("FormLinkInt", p.FunctionType.ToString(), Values(FormKeyOrNull(p.Record), p.Value)),
                IObjectModFormLinkFloatPropertyGetter<TProperty> p => ("FormLinkFloat", p.FunctionType.ToString(), Values(FormKeyOrNull(p.Record), p.Value)),
                _ => throw new NotSupportedException($"Unsupported object modification property {property.GetType().Name}"),
            };
            return new JsonObject
            {
                ["value_type"] = valueType,
                ["property"] = property.Property.ToString(),
                ["function"] = function,
                ["step"] = property.Step,
                ["values"] = values,
            };
        }

        static string? FormKeyOrNull(IFormLinkGetter link) => link.IsNull ? null : link.FormKey.ToString();

        static JsonArray Values(params object?[] values) =>
            new(values.Select(value => (JsonNode?)(value == null ? null : JsonValue.Create(value))).ToArray());

        public static string? TargetType(IAObjectModificationGetter record) => record switch
        {
            IWeaponModificationGetter => "WEAP",
            IArmorModificationGetter => "ARMO",
            INpcModificationGetter => "NPC_",
            _ => null,
        };

        public static string? ModProperties(IAObjectModificationGetter record) => record switch
        {
            IWeaponModificationGetter weapon => Properties(weapon.Properties).ToJsonString(),
            IArmorModificationGetter armor => Properties(armor.Properties).ToJsonString(),
            INpcModificationGetter npc => Properties(npc.Properties).ToJsonString(),
            _ => null,
        };

        // The OMOD job stores keyword form keys; the keyword names live in a table loaded concurrently
        // with it, so the editor ids are filled in once every record job has finished.
        public static async Task ResolveAttachPointNamesAsync(SqliteConnection conn, SqliteTransaction? transaction)
        {
            await conn.ExecuteAsync(@"
                UPDATE object_modifications SET attach_point_name = COALESCE(
                    (SELECT k.editor_id FROM keywords k WHERE k.form_key = object_modifications.attach_point),
                    attach_point)
                WHERE attach_point IS NOT NULL", transaction: transaction);
            await conn.ExecuteAsync(@"
                UPDATE object_modifications SET attach_parent_slots = (
                    SELECT json_group_array(name) FROM (
                        SELECT COALESCE(k.editor_id, slot.value) AS name
                        FROM json_each(object_modifications.attach_parent_slots) AS slot
                        LEFT JOIN keywords k ON k.form_key = slot.value
                        ORDER BY slot.key))
                WHERE attach_parent_slots IS NOT NULL", transaction: transaction);

            long unnamed = await conn.ExecuteScalarAsync<long>(@"
                SELECT COUNT(*) FROM object_modifications om
                WHERE om.attach_point IS NOT NULL
                  AND NOT EXISTS (SELECT 1 FROM keywords k WHERE k.form_key = om.attach_point AND k.editor_id IS NOT NULL)",
                transaction: transaction);
            if (unnamed > 0)
                Console.WriteLine($"  WARNING: {unnamed} object modification attach point(s) have no keyword editor id; stored the form key instead.");
        }
    }
}
