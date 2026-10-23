using Mutagen.Bethesda.Plugins;
using Mutagen.Bethesda.Plugins.Records;

namespace AssetConverter
{
    internal static class FormKeyStrings
    {
        // Mutagen FormKey.ToString() of every link in order; null for a missing list, empty for an empty one.
        public static List<string>? Of<T>(IEnumerable<IFormLinkGetter<T>>? links)
            where T : class, IMajorRecordGetter =>
            links?.Select(link => link.FormKey.ToString()).ToList();
    }
}
