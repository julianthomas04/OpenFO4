using SharpGLTF.Schema2;

namespace AssetConverter
{
    // The engine addresses every node of an animated or skinned asset by name, so
    // names must be unique there. Animated nodes keep their name, then mesh-free
    // nodes, then the earliest node; the others get a "~n" suffix.
    internal static class GltfNodeNames
    {
        public static void EnsureUnique(ModelRoot model)
        {
            if (model.LogicalAnimations.Count == 0 && model.LogicalSkins.Count == 0)
                return;

            foreach (var group in model.LogicalNodes.Where(n => !string.IsNullOrEmpty(n.Name)).GroupBy(n => n.Name).Where(g => g.Count() > 1))
            {
                var keeper = group.OrderBy(n => n.Extras == null ? 1 : 0).ThenBy(n => n.Mesh == null ? 0 : 1).ThenBy(n => n.LogicalIndex).First();
                int suffix = 2;
                foreach (var node in group.Where(n => n != keeper).OrderBy(n => n.LogicalIndex))
                {
                    node.Name = $"{group.Key}~{suffix++}";
                    NifAnimationStats.Add(NifAnimationStats.NodesRenamedForUniqueness);
                }
            }
        }
    }
}
