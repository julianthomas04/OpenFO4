namespace AssetConverter.Pipeline
{
    /// <summary>A set of converted assets that is kept or converted again as a whole when an install runs over an existing one.</summary>
    internal enum AssetGroup
    {
        Shaders,
        Interface,
        Audio,
        Video,
        Meshes,
        Collision,
        Animations,
        TexturesDatabase,
    }

    /// <summary>
    /// The one table that defines the asset groups: the id each is persisted under in the ownership marker, the revision of its converted
    /// output and the groups whose output is built from it. <b>Bump a group's revision whenever its converted output changes</b>, so
    /// installs made by an older installer convert that group (and, through the dependents, everything built from it) again.
    /// </summary>
    internal static class AssetGroups
    {
        private sealed record Entry(AssetGroup Group, string Id, int Revision, AssetGroup[] Dependents);

        private static readonly Entry[] Table =
        [
            new(AssetGroup.Shaders, "shaders", 1, []),
            new(AssetGroup.Interface, "interface", 1, []),
            new(AssetGroup.Audio, "audio", 1, []),
            new(AssetGroup.Video, "video", 1, []),
            new(AssetGroup.Meshes, "meshes", 1, [AssetGroup.Animations, AssetGroup.TexturesDatabase]),
            new(AssetGroup.Collision, "collision", 1, []),
            new(AssetGroup.Animations, "animations", 1, []),
            new(AssetGroup.TexturesDatabase, "textures-database", 1, []),
        ];

        private static readonly Dictionary<AssetGroup, Entry> ByGroup = Table.ToDictionary(entry => entry.Group);
        private static readonly Dictionary<string, AssetGroup> ByPersistedId = Table.ToDictionary(entry => entry.Id, entry => entry.Group);

        public static IReadOnlyList<AssetGroup> All { get; } = [.. Table.Select(entry => entry.Group)];

        /// <summary>The revision of every group as this installer converts it.</summary>
        public static IReadOnlyDictionary<AssetGroup, int> CurrentRevisions { get; } = Table.ToDictionary(entry => entry.Group, entry => entry.Revision);

        public static string Id(AssetGroup group) => ByGroup[group].Id;

        public static bool TryFromId(string id, out AssetGroup group) => ByPersistedId.TryGetValue(id, out group);

        /// <summary>The given groups plus every group built from them, directly or through others.</summary>
        public static IReadOnlySet<AssetGroup> WithDependents(IEnumerable<AssetGroup> groups)
        {
            var closure = new HashSet<AssetGroup>();
            var pending = new Stack<AssetGroup>(groups);
            while (pending.TryPop(out var group))
            {
                if (!closure.Add(group))
                    continue;
                foreach (var dependent in ByGroup[group].Dependents)
                    pending.Push(dependent);
            }
            return closure;
        }
    }
}
