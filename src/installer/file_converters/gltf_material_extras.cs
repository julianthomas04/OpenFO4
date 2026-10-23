using System.Buffers.Binary;
using System.Text;
using System.Text.Json.Nodes;
using SharpGLTF.Materials;

namespace AssetConverter
{
    /// <summary>Writes the engine-facing material extras (<c>effect</c>, <c>refraction</c>) and derives the cache key that goes into the material name.</summary>
    internal static class GltfMaterialExtras
    {
        public const string EffectKind = "effect";
        public const string RefractionKind = "refraction";

        public static void Attach(MaterialBuilder material, string kind, JsonObject extras) =>
            material.Extras = new JsonObject { [kind] = extras.DeepClone() };

        /// <summary>Short xxh32 of the extras' compact JSON for the material name; property order is fixed by the builders, so equal parameters give equal hashes.</summary>
        public static uint KeyOf(JsonObject extras) => Xxh32.Hash(Encoding.UTF8.GetBytes(extras.ToJsonString()));

        /// <summary>Material cache key: the name plus the canonical extras JSON, so a hash collision in the name never merges two materials.</summary>
        public static string CacheKey(string materialName, JsonObject? extras) =>
            extras == null ? materialName : $"{materialName}\n{extras.ToJsonString()}";
    }

    /// <summary>XXH32 (seed 0), the 32-bit xxHash.</summary>
    internal static class Xxh32
    {
        private const uint Prime1 = 2654435761;
        private const uint Prime2 = 2246822519;
        private const uint Prime3 = 3266489917;
        private const uint Prime4 = 668265263;
        private const uint Prime5 = 374761393;

        public static uint Hash(ReadOnlySpan<byte> data)
        {
            int offset = 0;
            uint hash;
            if (data.Length >= 16)
            {
                uint v1 = unchecked(Prime1 + Prime2), v2 = Prime2, v3 = 0, v4 = unchecked(0 - Prime1);
                for (; offset <= data.Length - 16; offset += 16)
                {
                    v1 = Round(v1, Read(data, offset));
                    v2 = Round(v2, Read(data, offset + 4));
                    v3 = Round(v3, Read(data, offset + 8));
                    v4 = Round(v4, Read(data, offset + 12));
                }
                hash = uint.RotateLeft(v1, 1) + uint.RotateLeft(v2, 7) + uint.RotateLeft(v3, 12) + uint.RotateLeft(v4, 18);
            }
            else
            {
                hash = Prime5;
            }

            hash += (uint)data.Length;
            for (; offset <= data.Length - 4; offset += 4)
                hash = uint.RotateLeft(hash + Read(data, offset) * Prime3, 17) * Prime4;
            for (; offset < data.Length; offset++)
                hash = uint.RotateLeft(hash + data[offset] * Prime5, 11) * Prime1;

            hash ^= hash >> 15;
            hash *= Prime2;
            hash ^= hash >> 13;
            hash *= Prime3;
            hash ^= hash >> 16;
            return hash;
        }

        private static uint Round(uint accumulator, uint input) => uint.RotateLeft(accumulator + input * Prime2, 13) * Prime1;

        private static uint Read(ReadOnlySpan<byte> data, int offset) => BinaryPrimitives.ReadUInt32LittleEndian(data[offset..]);
    }
}
