using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;

namespace AssetConverter
{
    /// <summary>One SPECIAL attribute as the Pip-Boy shows it: lowercase key, the record and its authored texts.</summary>
    internal sealed record SpecialAttributeEntry(string Special, FormKey Record, string? Name, string? Description);

    /// <summary>The seven SPECIAL attributes, identified by the editor id of their actor value record.</summary>
    internal static class SpecialAttributes
    {
        static readonly string[] EditorIds = ["Strength", "Perception", "Endurance", "Charisma", "Intelligence", "Agility", "Luck"];

        public static IReadOnlyList<string> Keys { get; } = EditorIds.Select(id => id.ToLowerInvariant()).ToList();

        /// <summary>Record form key to lowercase attribute key.</summary>
        public static Dictionary<FormKey, string> KeysByRecord(IEnumerable<IActorValueInformationGetter> actorValues) =>
            actorValues
                .Where(actorValue => actorValue.EditorID is not null && EditorIds.Contains(actorValue.EditorID, StringComparer.OrdinalIgnoreCase))
                .ToDictionary(actorValue => actorValue.FormKey, actorValue => actorValue.EditorID!.ToLowerInvariant());

        public static IReadOnlyList<SpecialAttributeEntry> Collect(IReadOnlyCollection<IActorValueInformationGetter> actorValues)
        {
            var keys = KeysByRecord(actorValues);
            var entries = actorValues
                .Where(actorValue => keys.ContainsKey(actorValue.FormKey))
                .Select(actorValue => new SpecialAttributeEntry(
                    keys[actorValue.FormKey], actorValue.FormKey, actorValue.Name?.ToString(), actorValue.Description?.ToString()))
                .OrderBy(entry => entry.Special, StringComparer.Ordinal)
                .ToList();
            var missing = Keys.Except(entries.Select(entry => entry.Special)).ToList();
            if (entries.Count > 0 && missing.Count > 0)
                throw new InvalidDataException($"No actor value record for SPECIAL attribute(s): {string.Join(", ", missing)}.");
            return entries;
        }
    }
}
