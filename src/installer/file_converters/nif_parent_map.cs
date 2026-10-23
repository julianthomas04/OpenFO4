using NiflySharp;
using NiflySharp.Blocks;

namespace AssetConverter
{
    /// <summary>Child block index to parent NiNode block index; the first parent in file order wins for a block listed under several nodes.</summary>
    internal static class NifParentMap
    {
        public static Dictionary<int, int> Build(NifFile nif)
        {
            var parentOf = new Dictionary<int, int>();
            for (int i = 0; i < nif.Blocks.Count; i++)
            {
                if (nif.Blocks[i] is not NiNode { Children.Indices: { } children })
                    continue;
                foreach (int child in children)
                    parentOf.TryAdd(child, i);
            }
            return parentOf;
        }
    }
}
