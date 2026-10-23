using NiflySharp;
using NiflySharp.Blocks;

namespace AssetConverter
{
    /// <summary>The glTF node name of a NIF block: its own name, or the generated one the converter gives unnamed blocks.</summary>
    internal static class NifNodeNames
    {
        public static string Of(NifFile nif, int block)
        {
            var obj = nif.Blocks[block];
            string name = (obj as NiObjectNET)?.Name?.String ?? "";
            if (name.Length > 0)
                return name;
            return obj is NiNode ? $"Node_{block}" : $"{obj.GetType().Name}_{block}";
        }
    }
}
