using System.IO.Compression;
using System.Text;

namespace InstallerTests.Archives
{
    /// <summary>Unique scratch directory removed on dispose.</summary>
    internal sealed class ArchiveTempDir : IDisposable
    {
        public string Path { get; } = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "of4-tests-" + Guid.NewGuid().ToString("N"));

        private readonly List<SharpBSABA2.Archive> _opened = new();

        public ArchiveTempDir() => Directory.CreateDirectory(Path);

        public SharpBSABA2.BA2Util.BA2 OpenBa2(string fileName, byte[] bytes)
        {
            var archive = new SharpBSABA2.BA2Util.BA2(Write(fileName, bytes));
            _opened.Add(archive);
            return archive;
        }

        public SharpBSABA2.BSAUtil.BSA OpenBsa(string fileName, byte[] bytes)
        {
            var archive = new SharpBSABA2.BSAUtil.BSA(Write(fileName, bytes));
            _opened.Add(archive);
            return archive;
        }

        public string Write(string fileName, byte[] bytes)
        {
            string full = System.IO.Path.Combine(Path, fileName);
            File.WriteAllBytes(full, bytes);
            return full;
        }

        public void Dispose()
        {
            foreach (var archive in _opened)
                archive.Close();
            Directory.Delete(Path, recursive: true);
        }
    }

    internal static class TestBytes
    {
        /// <summary>Deterministic, poorly compressible pseudo-random payload.</summary>
        public static byte[] Random(int length, int seed)
        {
            var bytes = new byte[length];
            new Random(seed).NextBytes(bytes);
            return bytes;
        }

        /// <summary>Highly compressible payload.</summary>
        public static byte[] Repeating(int length, byte value) => Enumerable.Repeat(value, length).ToArray();

        public static byte[] Zlib(byte[] data)
        {
            using var output = new MemoryStream();
            using (var z = new ZLibStream(output, CompressionLevel.Optimal, leaveOpen: true))
                z.Write(data);
            return output.ToArray();
        }
    }

    internal sealed record Ba2File(string Name, byte[] Content, bool Compress);

    internal sealed record Ba2Chunk(byte[] Data, bool Compress, ushort StartMip = 0, ushort EndMip = 0);

    internal sealed record Ba2Texture(string Name, ushort Width, ushort Height, byte Mips, byte Format, IReadOnlyList<Ba2Chunk> Chunks,
        bool Cubemap = false, byte TileMode = 8);

    /// <summary>
    /// Writes BA2 archives following the BA2 format: 24-byte header (32 with the two extra u32 for version 2 and 3),
    /// the entry table, the data blocks and the name table (int16 length + Latin-1 bytes).
    /// </summary>
    internal static class Ba2Builder
    {
        private const int GnrlEntrySize = 36;
        private const int Dx10EntryPrefixSize = 24;
        private const int Dx10ChunkSize = 24;
        private const uint Align = 0xBAADF00D;

        public static byte[] Gnrl(IReadOnlyList<Ba2File> files, uint version = 1, bool nameTable = true, bool reverseDataOrder = false)
        {
            int headerSize = HeaderSize(version);
            long dataStart = headerSize + (long)files.Count * GnrlEntrySize;

            var blobs = files.Select(f => f.Compress ? TestBytes.Zlib(f.Content) : f.Content).ToList();
            var offsets = DataOffsets(blobs, dataStart, reverseDataOrder);
            long nameTableOffset = dataStart + blobs.Sum(b => (long)b.Length);

            using var ms = new MemoryStream();
            using var bw = new BinaryWriter(ms, Encoding.Latin1);
            WriteHeader(bw, version, "GNRL", files.Count, nameTable ? nameTableOffset : 0);
            for (int i = 0; i < files.Count; i++)
            {
                bw.Write(NameHash(i));
                bw.Write(Extension(files[i].Name));
                bw.Write(DirHash(i));
                bw.Write(0u);
                bw.Write((ulong)offsets[i]);
                bw.Write(files[i].Compress ? (uint)blobs[i].Length : 0u);
                bw.Write((uint)files[i].Content.Length);
                bw.Write(Align);
            }
            WriteBlobs(bw, blobs, offsets);
            if (nameTable) WriteNameTable(bw, files.Select(f => f.Name));
            return ms.ToArray();
        }

        public static byte[] Dx10(IReadOnlyList<Ba2Texture> textures, uint version = 1, bool nameTable = true)
        {
            int headerSize = HeaderSize(version);
            long entriesSize = textures.Sum(t => (long)Dx10EntryPrefixSize + t.Chunks.Count * Dx10ChunkSize);
            long dataStart = headerSize + entriesSize;

            var blobs = textures.SelectMany(t => t.Chunks).Select(c => c.Compress ? TestBytes.Zlib(c.Data) : c.Data).ToList();
            var offsets = DataOffsets(blobs, dataStart, reverse: false);
            long nameTableOffset = dataStart + blobs.Sum(b => (long)b.Length);

            using var ms = new MemoryStream();
            using var bw = new BinaryWriter(ms, Encoding.Latin1);
            WriteHeader(bw, version, "DX10", textures.Count, nameTable ? nameTableOffset : 0);
            int chunkIndex = 0;
            for (int i = 0; i < textures.Count; i++)
            {
                var t = textures[i];
                bw.Write(NameHash(i));
                bw.Write(Extension(t.Name));
                bw.Write(DirHash(i));
                bw.Write((byte)0);
                bw.Write((byte)t.Chunks.Count);
                bw.Write((ushort)Dx10ChunkSize);
                bw.Write(t.Height);
                bw.Write(t.Width);
                bw.Write(t.Mips);
                bw.Write(t.Format);
                bw.Write((byte)(t.Cubemap ? 1 : 0));
                bw.Write(t.TileMode);
                foreach (var chunk in t.Chunks)
                {
                    bw.Write((ulong)offsets[chunkIndex]);
                    bw.Write(chunk.Compress ? (uint)blobs[chunkIndex].Length : 0u);
                    bw.Write((uint)chunk.Data.Length);
                    bw.Write(chunk.StartMip);
                    bw.Write(chunk.EndMip);
                    bw.Write(Align);
                    chunkIndex++;
                }
            }
            WriteBlobs(bw, blobs, offsets);
            if (nameTable) WriteNameTable(bw, textures.Select(t => t.Name));
            return ms.ToArray();
        }

        /// <summary>Header only, with caller-controlled fields, for malformed-input tests.</summary>
        public static byte[] RawHeader(string magic, uint version, string type, uint numFiles, ulong nameTableOffset)
        {
            using var ms = new MemoryStream();
            using var bw = new BinaryWriter(ms, Encoding.Latin1);
            bw.Write(Encoding.Latin1.GetBytes(magic));
            bw.Write(version);
            bw.Write(Encoding.Latin1.GetBytes(type));
            bw.Write(numFiles);
            bw.Write(nameTableOffset);
            if (HeaderSize(version) == 32)
            {
                bw.Write(0u);
                bw.Write(0u);
            }
            return ms.ToArray();
        }

        public static uint NameHash(int index) => 0x1000u + (uint)index;

        public static uint DirHash(int index) => 0xD000u + (uint)index;

        private static int HeaderSize(uint version) => version is >= 2 and <= 3 ? 32 : 24;

        private static void WriteHeader(BinaryWriter bw, uint version, string type, int count, long nameTableOffset)
        {
            bw.Write(Encoding.Latin1.GetBytes("BTDX"));
            bw.Write(version);
            bw.Write(Encoding.Latin1.GetBytes(type));
            bw.Write((uint)count);
            bw.Write((ulong)nameTableOffset);
            if (HeaderSize(version) == 32)
            {
                bw.Write(0u);
                bw.Write(0u);
            }
        }

        private static byte[] Extension(string name)
        {
            string ext = Path.GetExtension(name).TrimStart('.');
            var bytes = new byte[4];
            Encoding.Latin1.GetBytes(ext, 0, Math.Min(4, ext.Length), bytes, 0);
            return bytes;
        }

        private static long[] DataOffsets(List<byte[]> blobs, long dataStart, bool reverse)
        {
            var offsets = new long[blobs.Count];
            long cursor = dataStart;
            var order = reverse ? Enumerable.Range(0, blobs.Count).Reverse() : Enumerable.Range(0, blobs.Count);
            foreach (int i in order)
            {
                offsets[i] = cursor;
                cursor += blobs[i].Length;
            }
            return offsets;
        }

        private static void WriteBlobs(BinaryWriter bw, List<byte[]> blobs, long[] offsets)
        {
            foreach (int i in Enumerable.Range(0, blobs.Count).OrderBy(i => offsets[i]))
                bw.Write(blobs[i]);
        }

        private static void WriteNameTable(BinaryWriter bw, IEnumerable<string> names)
        {
            foreach (string name in names)
            {
                var bytes = Encoding.Latin1.GetBytes(name);
                bw.Write((short)bytes.Length);
                bw.Write(bytes);
            }
        }
    }

    internal sealed record BsaFile(string Name, byte[] Content);

    internal sealed record BsaFolder(string Name, IReadOnlyList<BsaFile> Files);

    /// <summary>
    /// Writes Oblivion-family BSA archives (header, folder records, file record blocks, file name block, data):
    /// version 0x68 (Fallout 3 / New Vegas, zlib).
    /// </summary>
    internal static class BsaBuilder
    {
        public const uint FalloutVersion = 0x68;
        private const uint IncludeDirectoryNames = 0x1;
        private const uint IncludeFileNames = 0x2;
        private const uint CompressedFlag = 0x4;
        private const int HeaderSize = 36;

        public static byte[] Build(IReadOnlyList<BsaFolder> folders, uint version, bool compressed)
        {
            const int folderRecordSize = 16;
            int fileCount = folders.Sum(f => f.Files.Count);
            int totalFolderNames = folders.Sum(f => f.Name.Length + 1);
            int totalFileNames = folders.Sum(f => f.Files.Sum(x => x.Name.Length + 1));

            long blockStart = HeaderSize + (long)folders.Count * folderRecordSize;
            long blocksSize = folders.Sum(f => 1 + f.Name.Length + 1 + (long)f.Files.Count * 16);
            long dataStart = blockStart + blocksSize + totalFileNames;

            var blobs = folders.SelectMany(f => f.Files).Select(f => compressed ? CompressedBlob(f.Content) : f.Content).ToList();
            var offsets = new List<long>();
            long cursor = dataStart;
            foreach (var blob in blobs)
            {
                offsets.Add(cursor);
                cursor += blob.Length;
            }

            using var ms = new MemoryStream();
            using var bw = new BinaryWriter(ms, Encoding.Latin1);
            bw.Write(Encoding.Latin1.GetBytes("BSA "));
            bw.Write(version);
            bw.Write((uint)HeaderSize);
            bw.Write(IncludeDirectoryNames | IncludeFileNames | (compressed ? CompressedFlag : 0));
            bw.Write((uint)folders.Count);
            bw.Write((uint)fileCount);
            bw.Write((uint)totalFolderNames);
            bw.Write((uint)totalFileNames);
            bw.Write(0u);

            long blockPointer = blockStart + totalFileNames;
            foreach (var folder in folders)
            {
                bw.Write(0xF000UL + (ulong)folder.Name.Length);
                bw.Write((uint)folder.Files.Count);
                bw.Write((uint)blockPointer);
                blockPointer += 1 + folder.Name.Length + 1 + (long)folder.Files.Count * 16;
            }

            int fileIndex = 0;
            foreach (var folder in folders)
            {
                bw.Write((byte)(folder.Name.Length + 1));
                bw.Write(Encoding.Latin1.GetBytes(folder.Name));
                bw.Write((byte)0);
                foreach (var file in folder.Files)
                {
                    bw.Write(0xA000UL + (ulong)fileIndex);
                    bw.Write((uint)blobs[fileIndex].Length);
                    bw.Write((uint)offsets[fileIndex]);
                    fileIndex++;
                }
            }

            foreach (var file in folders.SelectMany(f => f.Files))
            {
                bw.Write(Encoding.Latin1.GetBytes(file.Name));
                bw.Write((byte)0);
            }

            foreach (var blob in blobs)
                bw.Write(blob);
            return ms.ToArray();
        }

        private static byte[] CompressedBlob(byte[] content)
        {
            using var ms = new MemoryStream();
            using var bw = new BinaryWriter(ms);
            bw.Write((uint)content.Length);
            bw.Write(TestBytes.Zlib(content));
            return ms.ToArray();
        }
    }
}
