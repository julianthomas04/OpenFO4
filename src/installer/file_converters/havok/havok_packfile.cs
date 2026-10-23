namespace AssetConverter.Havok
{
    public sealed record HavokSection(string Name, int Start, int DataLength);

    public sealed record HavokObject(string ClassName, int Offset);

    public readonly record struct HavokArrayRef(int DataOffset, int Count);

    /// <summary>
    /// Havok 2014.1 64-bit binary packfile (Fallout 4): header, sections and fixup tables.
    /// All offsets handed out are absolute file offsets.
    /// </summary>
    public sealed class HavokPackfile
    {
        private const int GlobalHeaderSize = 0x40;
        private const int SectionHeaderSize = 0x40;
        private const int PointerSize = 8;
        private const int ExpectedFileVersion = 11;
        private const string ExpectedContentsVersionPrefix = "hk_2014.1.0-r1";
        private const int MaxSections = 16;
        private const int MaxNameLength = 256;
        private const uint FixupTerminator = 0xFFFFFFFF;
        private const string DataSectionName = "__data__";
        private const string ClassNamesSectionName = "__classnames__";

        private static readonly byte[] Magic = { 0x57, 0xE0, 0xE0, 0x57, 0x10, 0xC0, 0xC0, 0x10 };

        private readonly Dictionary<int, int> _pointers = new();
        private readonly List<HavokObject> _objects = new();
        private readonly Dictionary<int, HavokObject> _objectsByOffset = new();

        public HavokBytes Bytes { get; }
        public string ContentsVersion { get; private set; } = "";
        public IReadOnlyList<HavokSection> Sections { get; private set; } = Array.Empty<HavokSection>();
        public IReadOnlyList<HavokObject> Objects => _objects;

        private HavokPackfile(HavokBytes bytes)
        {
            Bytes = bytes;
        }

        public static HavokPackfile Load(byte[] data)
        {
            var file = new HavokPackfile(new HavokBytes(data));
            file.ParseHeader();
            return file;
        }

        public IEnumerable<HavokObject> ObjectsOfClass(string className) =>
            _objects.Where(o => o.ClassName == className);

        public HavokObject RequireObjectAt(int offset, string what)
        {
            if (!_objectsByOffset.TryGetValue(offset, out var obj))
                throw new HavokFormatException($"{what}: no object registered at offset {offset}");
            return obj;
        }

        /// <summary>Resolves the pointer stored at <paramref name="fieldOffset"/> through the fixup tables; null for a null pointer.</summary>
        public int? ReadPointer(int fieldOffset)
        {
            Bytes.Require(fieldOffset, PointerSize, "pointer field");
            if (_pointers.TryGetValue(fieldOffset, out int target))
                return target;
            if (Bytes.U64(fieldOffset) != 0)
                throw new HavokFormatException($"Non-zero pointer at offset {fieldOffset} has no fixup");
            return null;
        }

        public int RequirePointer(int fieldOffset, string what) =>
            ReadPointer(fieldOffset) ?? throw new HavokFormatException($"{what}: unexpected null pointer at offset {fieldOffset}");

        public HavokObject RequireObjectPointer(int fieldOffset, string expectedClass, string what)
        {
            var obj = RequireObjectAt(RequirePointer(fieldOffset, what), what);
            if (obj.ClassName != expectedClass)
                throw new HavokUnsupportedClassException(what, obj.ClassName);
            return obj;
        }

        public string? ReadString(int fieldOffset)
        {
            int? target = ReadPointer(fieldOffset);
            return target is null ? null : Bytes.CString(target.Value, MaxNameLength, "string");
        }

        /// <summary>hkArray: pointer, size, capacity|flags. Validates the payload fits the buffer.</summary>
        public HavokArrayRef ReadArray(int fieldOffset, int elementSize, string what)
        {
            uint count = Bytes.U32(fieldOffset + PointerSize);
            if (count == 0)
                return new HavokArrayRef(0, 0);

            int data = RequirePointer(fieldOffset, what);
            long byteLength = (long)count * elementSize;
            if (count > int.MaxValue || byteLength > Bytes.Length)
                throw new HavokFormatException($"{what}: array of {count} x {elementSize} bytes exceeds buffer");
            Bytes.Require(data, byteLength, what);
            return new HavokArrayRef(data, (int)count);
        }

        private void ParseHeader()
        {
            var b = Bytes;
            b.Require(0, GlobalHeaderSize, "global header");
            if (!b.Slice(0, Magic.Length, "magic").SequenceEqual(Magic))
                throw new HavokFormatException("Not a Havok packfile: bad magic");

            int fileVersion = b.I32(0x0C);
            if (fileVersion != ExpectedFileVersion)
                throw new HavokFormatException($"Unsupported packfile version {fileVersion}, expected {ExpectedFileVersion}");

            if (b.U8(0x10) != PointerSize)
                throw new HavokFormatException($"Unsupported pointer size {b.U8(0x10)}, expected {PointerSize}");
            if (b.U8(0x11) != 1)
                throw new HavokFormatException("Only little-endian packfiles are supported");

            int sectionCount = b.I32(0x14);
            if (sectionCount < 1 || sectionCount > MaxSections)
                throw new HavokFormatException($"Implausible section count {sectionCount}");

            ContentsVersion = b.CString(0x28, 16, "contents version");
            if (!ContentsVersion.StartsWith(ExpectedContentsVersionPrefix, StringComparison.Ordinal))
                throw new HavokFormatException($"Unsupported Havok version '{ContentsVersion}', expected '{ExpectedContentsVersionPrefix}'");

            int headerPadding = b.U16(0x3E);
            var raw = new List<RawSection>();
            for (int i = 0; i < sectionCount; i++)
                raw.Add(ReadSectionHeader(GlobalHeaderSize + headerPadding + i * SectionHeaderSize));

            Sections = raw.Select(s => new HavokSection(s.Name, s.Start, (int)s.DataLength)).ToList();
            foreach (var section in raw)
                ReadFixups(section, raw);
        }

        private RawSection ReadSectionHeader(int offset)
        {
            var b = Bytes;
            b.Require(offset, SectionHeaderSize, "section header");
            string name = b.CString(offset, 19, "section name");
            uint rawStart = b.U32(offset + 0x14);
            if (rawStart > int.MaxValue)
                throw new HavokFormatException($"Section '{name}' starts at offset {rawStart}, beyond any addressable buffer");
            int start = (int)rawStart;
            var section = new RawSection(
                name, start,
                LocalFixups: b.U32(offset + 0x18),
                GlobalFixups: b.U32(offset + 0x1C),
                VirtualFixups: b.U32(offset + 0x20),
                Exports: b.U32(offset + 0x24));
            b.Require(start, section.DataLength, $"section '{name}' data");
            if (section.LocalFixups < section.DataLength || section.GlobalFixups < section.LocalFixups
                || section.VirtualFixups < section.GlobalFixups || section.Exports < section.VirtualFixups)
                throw new HavokFormatException($"Section '{name}' has inconsistent fixup table order");
            b.Require(start, section.Exports, $"section '{name}' fixups");
            return section;
        }

        private void ReadFixups(RawSection section, List<RawSection> all)
        {
            ReadLocalFixups(section);
            ReadGlobalFixups(section, all);
            if (section.Name == DataSectionName)
                ReadVirtualFixups(section, all);
        }

        private void ReadLocalFixups(RawSection section)
        {
            for (long pos = section.Start + section.LocalFixups; pos + 8 <= section.Start + section.GlobalFixups; pos += 8)
            {
                uint src = Bytes.U32(pos);
                if (src == FixupTerminator) break;
                uint dst = Bytes.U32(pos + 4);
                CheckInSection(section, src, PointerSize, "local fixup source");
                CheckInSection(section, dst, 0, "local fixup target");
                _pointers[section.Start + (int)src] = section.Start + (int)dst;
            }
        }

        private void ReadGlobalFixups(RawSection section, List<RawSection> all)
        {
            for (long pos = section.Start + section.GlobalFixups; pos + 12 <= section.Start + section.VirtualFixups; pos += 12)
            {
                uint src = Bytes.U32(pos);
                if (src == FixupTerminator) break;
                var target = SectionByIndex(all, Bytes.U32(pos + 4), "global fixup");
                uint dst = Bytes.U32(pos + 8);
                CheckInSection(section, src, PointerSize, "global fixup source");
                CheckInSection(target, dst, 0, "global fixup target");
                _pointers[section.Start + (int)src] = target.Start + (int)dst;
            }
        }

        private void ReadVirtualFixups(RawSection section, List<RawSection> all)
        {
            for (long pos = section.Start + section.VirtualFixups; pos + 12 <= section.Start + section.Exports; pos += 12)
            {
                uint src = Bytes.U32(pos);
                if (src == FixupTerminator) break;
                var classNames = SectionByIndex(all, Bytes.U32(pos + 4), "virtual fixup");
                if (classNames.Name != ClassNamesSectionName)
                    throw new HavokFormatException($"Virtual fixup names section '{classNames.Name}', expected '{ClassNamesSectionName}'");
                uint nameOffset = Bytes.U32(pos + 8);
                CheckInSection(section, src, 0, "virtual fixup object");
                CheckInSection(classNames, nameOffset, 1, "virtual fixup class name");

                string className = Bytes.CString(classNames.Start + nameOffset, MaxNameLength, "class name");
                var obj = new HavokObject(className, section.Start + (int)src);
                _objects.Add(obj);
                _objectsByOffset[obj.Offset] = obj;
            }
            _objects.Sort((a, c) => a.Offset.CompareTo(c.Offset));
        }

        private static RawSection SectionByIndex(List<RawSection> all, uint index, string what)
        {
            if (index >= all.Count)
                throw new HavokFormatException($"{what}: section index {index} out of range");
            return all[(int)index];
        }

        private static void CheckInSection(RawSection section, uint offset, int length, string what)
        {
            if ((long)offset + length > section.DataLength)
                throw new HavokFormatException($"{what}: offset {offset} (+{length}) outside section '{section.Name}' of {section.DataLength} bytes");
        }

        private sealed record RawSection(string Name, int Start, uint LocalFixups, uint GlobalFixups, uint VirtualFixups, uint Exports)
        {
            public uint DataLength => LocalFixups;
        }
    }
}
