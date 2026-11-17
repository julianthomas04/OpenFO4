using Mutagen.Bethesda.Fallout4;

namespace AssetConverter
{
    internal static class OwnerKeys
    {
        /// <summary>
        /// The form key of the NPC or faction that owns a reference or cell; null when nothing owns it. The required faction rank is not kept.
        /// </summary>
        public static string? Of(IOwnershipGetter? ownership) =>
            ownership == null || ownership.Owner.IsNull ? null : ownership.Owner.FormKey.ToString();
    }
}
