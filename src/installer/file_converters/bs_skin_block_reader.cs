using System.Numerics;
using NiflySharp;
using NiflySharp.Blocks;
using NiflySharp.Structs;

namespace AssetConverter
{
    // NiflySharp does not map the "BSSkin::Instance" / "BSSkin::BoneData" block
    // type names to its BSSkin_* classes, so both arrive as NiUnknown with their raw
    // payload. Layouts follow nif.xml (FO4 and later).
    internal static class BsSkinBlockReader
    {
        public const string InstanceTypeName = "BSSkin::Instance";
        public const string BoneDataTypeName = "BSSkin::BoneData";

        private const int BoneTransformSize = 68;

        internal sealed record BoneTransform(Matrix33 Rotation, Vector3 Translation, float Scale);

        public static string TypeName(NifFile nif, int blockIndex)
            => nif.Header.GetBlockTypeNameById(blockIndex);

        public static int[] ReadInstanceBones(NifFile nif, int blockIndex, out int boneDataRef)
        {
            var reader = Open(nif, blockIndex, InstanceTypeName);
            reader.ReadInt32();
            boneDataRef = reader.ReadInt32();
            int count = CheckedCount(reader.ReadUInt32(), reader.BaseStream.Length - reader.BaseStream.Position, sizeof(int), InstanceTypeName);
            var bones = new int[count];
            for (int i = 0; i < count; i++)
                bones[i] = reader.ReadInt32();
            return bones;
        }

        public static BoneTransform[] ReadBoneTransforms(NifFile nif, int blockIndex)
        {
            var reader = Open(nif, blockIndex, BoneDataTypeName);
            int count = CheckedCount(reader.ReadUInt32(), reader.BaseStream.Length - reader.BaseStream.Position, BoneTransformSize, BoneDataTypeName);
            var transforms = new BoneTransform[count];
            for (int i = 0; i < count; i++)
            {
                reader.BaseStream.Seek(16, SeekOrigin.Current); // bounding sphere
                var rotation = new Matrix33
                {
                    M11 = reader.ReadSingle(),
                    M21 = reader.ReadSingle(),
                    M31 = reader.ReadSingle(),
                    M12 = reader.ReadSingle(),
                    M22 = reader.ReadSingle(),
                    M32 = reader.ReadSingle(),
                    M13 = reader.ReadSingle(),
                    M23 = reader.ReadSingle(),
                    M33 = reader.ReadSingle(),
                };
                var translation = new Vector3(reader.ReadSingle(), reader.ReadSingle(), reader.ReadSingle());
                transforms[i] = new BoneTransform(rotation, translation, reader.ReadSingle());
            }
            return transforms;
        }

        private static BinaryReader Open(NifFile nif, int blockIndex, string expectedTypeName)
        {
            string actual = TypeName(nif, blockIndex);
            if (actual != expectedTypeName)
                throw new NifSkinException($"block {blockIndex} is {actual}, expected {expectedTypeName}");
            if (nif.Blocks[blockIndex] is not NiUnknown raw || raw.Data == null)
                throw new NifSkinException($"block {blockIndex} ({expectedTypeName}) was parsed as {nif.Blocks[blockIndex]?.GetType().Name} and has no raw payload");
            return new BinaryReader(new MemoryStream(raw.Data, writable: false));
        }

        private static int CheckedCount(uint count, long remainingBytes, int elementSize, string typeName)
        {
            if (count > remainingBytes / elementSize)
                throw new NifSkinException($"{typeName} claims {count} elements but only {remainingBytes} bytes remain");
            return (int)count;
        }
    }
}
