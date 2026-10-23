using System.Text;

namespace AssetConverter.Abc
{
    /// <summary>Reads an ABC (AVM2 bytecode) block as the DoABC tag carries it. Malformed data throws <see cref="AbcFormatException"/>.</summary>
    internal static class AbcReader
    {
        private const int TraitAttributeMetadata = 0x4;
        private const int InstanceProtectedNamespace = 0x08;

        public static AbcFile Read(byte[] data)
        {
            var bytes = new AbcBytes(data, 0, data.Length);
            bytes.U16();
            bytes.U16();

            var ints = Pool(bytes, () => bytes.S32(), 0);
            var uints = Pool(bytes, () => bytes.U32(), 0u);
            var doubles = Pool(bytes, () => bytes.Double(), double.NaN);
            var strings = Pool<string?>(bytes, () => bytes.Utf8(), null);
            var namespaces = Pool<AbcNamespace?>(bytes, () => ReadNamespace(bytes, strings), null);
            var namespaceSets = Pool<IReadOnlyList<int>?>(bytes, () => Repeat(bytes, () => bytes.U30()), null);
            var multinames = Pool<AbcMultiname?>(bytes, () => ReadMultiname(bytes, strings, namespaces, namespaceSets), null);

            var methods = Repeat(bytes, () => ReadMethod(bytes, strings));
            SkipMetadata(bytes);
            int classCount = bytes.U30();
            bytes.RequireEntries(classCount);
            var instances = new List<AbcInstance>(classCount);
            for (int i = 0; i < classCount; i++)
                instances.Add(ReadInstance(bytes));
            var classes = new List<AbcClass>(classCount);
            for (int i = 0; i < classCount; i++)
                classes.Add(new AbcClass(bytes.U30(), ReadTraits(bytes)));
            var scripts = Repeat(bytes, () => new AbcScript(bytes.U30(), ReadTraits(bytes)));
            var bodies = Repeat(bytes, () => ReadBody(bytes));

            return new AbcFile(ints, uints, doubles, strings, namespaces, multinames, methods, instances, classes, scripts, bodies);
        }

        /// <summary>A constant pool: its count includes the implicit entry 0, so a count of 0 or 1 holds no entries.</summary>
        private static List<T> Pool<T>(AbcBytes bytes, Func<T> read, T placeholder)
        {
            int count = bytes.U30();
            bytes.RequireEntries(Math.Max(count - 1, 0));
            var pool = new List<T>(Math.Max(count, 1)) { placeholder };
            for (int i = 1; i < count; i++)
                pool.Add(read());
            return pool;
        }

        private static List<T> Repeat<T>(AbcBytes bytes, Func<T> read)
        {
            int count = bytes.U30();
            bytes.RequireEntries(count);
            var items = new List<T>(count);
            for (int i = 0; i < count; i++)
                items.Add(read());
            return items;
        }

        private static AbcNamespace ReadNamespace(AbcBytes bytes, List<string?> strings)
        {
            int kind = bytes.U8();
            if (!Enum.IsDefined(typeof(AbcNamespaceKind), kind))
                throw new AbcFormatException($"Namespace kind 0x{kind:x2} is not an ABC namespace kind.");
            int name = bytes.U30();
            return new AbcNamespace((AbcNamespaceKind)kind, PoolEntry(strings, name, "string") ?? "");
        }

        private static AbcMultiname ReadMultiname(AbcBytes bytes, List<string?> strings, List<AbcNamespace?> namespaces, List<IReadOnlyList<int>?> namespaceSets)
        {
            int kind = bytes.U8();
            switch ((AbcMultinameKind)kind)
            {
                case AbcMultinameKind.QName:
                case AbcMultinameKind.QNameA:
                    {
                        int ns = bytes.U30();
                        int name = bytes.U30();
                        return new AbcMultiname((AbcMultinameKind)kind, PoolEntry(namespaces, ns, "namespace"), PoolEntry(strings, name, "string"));
                    }
                case AbcMultinameKind.RTQName:
                case AbcMultinameKind.RTQNameA:
                    return new AbcMultiname((AbcMultinameKind)kind, null, PoolEntry(strings, bytes.U30(), "string"));
                case AbcMultinameKind.RTQNameL:
                case AbcMultinameKind.RTQNameLA:
                    return new AbcMultiname((AbcMultinameKind)kind, null, null);
                case AbcMultinameKind.Multiname:
                case AbcMultinameKind.MultinameA:
                    {
                        int name = bytes.U30();
                        var lookedUp = NamespaceSet(namespaceSets, namespaces, bytes.U30());
                        return new AbcMultiname((AbcMultinameKind)kind, null, PoolEntry(strings, name, "string"), NamespaceSet: lookedUp);
                    }
                case AbcMultinameKind.MultinameL:
                case AbcMultinameKind.MultinameLA:
                    PoolEntry(namespaceSets, bytes.U30(), "namespace set");
                    return new AbcMultiname((AbcMultinameKind)kind, null, null);
                case AbcMultinameKind.TypeName:
                    {
                        int baseName = bytes.U30();
                        var parameters = Repeat(bytes, () => bytes.U30());
                        return new AbcMultiname(AbcMultinameKind.TypeName, null, null, baseName, parameters);
                    }
                default:
                    throw new AbcFormatException($"Multiname kind 0x{kind:x2} is not an ABC multiname kind.");
            }
        }

        private static List<AbcNamespace> NamespaceSet(List<IReadOnlyList<int>?> namespaceSets, List<AbcNamespace?> namespaces, int index) =>
            (PoolEntry(namespaceSets, index, "namespace set") ?? []).Select(member => PoolEntry(namespaces, member, "namespace") ?? throw new AbcFormatException($"A namespace set names the empty namespace {member}.")).ToList();

        private static T? PoolEntry<T>(List<T> pool, int index, string what) =>
            index < pool.Count ? pool[index] : throw new AbcFormatException($"The {what} index {index} is outside its pool of {pool.Count}.");

        private static AbcMethod ReadMethod(AbcBytes bytes, List<string?> strings)
        {
            int parameterCount = bytes.U30();
            bytes.U30();
            for (int i = 0; i < parameterCount; i++)
                bytes.U30();
            string name = PoolEntry(strings, bytes.U30(), "string") ?? "";
            var flags = (AbcMethodFlags)bytes.U8();
            if (flags.HasFlag(AbcMethodFlags.HasOptional))
            {
                int optionCount = bytes.U30();
                for (int i = 0; i < optionCount; i++)
                {
                    bytes.U30();
                    bytes.U8();
                }
            }
            if (flags.HasFlag(AbcMethodFlags.HasParamNames))
                for (int i = 0; i < parameterCount; i++)
                    bytes.U30();
            return new AbcMethod(parameterCount, name, flags);
        }

        private static void SkipMetadata(AbcBytes bytes)
        {
            int count = bytes.U30();
            for (int i = 0; i < count; i++)
            {
                bytes.U30();
                int items = bytes.U30();
                for (int j = 0; j < items * 2; j++)
                    bytes.U30();
            }
        }

        private static AbcInstance ReadInstance(AbcBytes bytes)
        {
            int name = bytes.U30();
            int superName = bytes.U30();
            int flags = bytes.U8();
            if ((flags & InstanceProtectedNamespace) != 0)
                bytes.U30();
            int interfaces = bytes.U30();
            for (int i = 0; i < interfaces; i++)
                bytes.U30();
            int initializer = bytes.U30();
            return new AbcInstance(name, superName, flags, initializer, ReadTraits(bytes));
        }

        private static List<AbcTrait> ReadTraits(AbcBytes bytes) => Repeat(bytes, () => ReadTrait(bytes));

        private static AbcTrait ReadTrait(AbcBytes bytes)
        {
            int name = bytes.U30();
            int kindByte = bytes.U8();
            int kind = kindByte & 0x0F;
            int attributes = kindByte >> 4;
            AbcTrait trait;
            switch ((AbcTraitKind)kind)
            {
                case AbcTraitKind.Slot:
                case AbcTraitKind.Const:
                    {
                        int slotId = bytes.U30();
                        bytes.U30();
                        int valueIndex = bytes.U30();
                        AbcValueRef? value = valueIndex == 0 ? null : new AbcValueRef(bytes.U8(), valueIndex);
                        trait = new AbcTrait(name, (AbcTraitKind)kind, slotId, value, 0);
                        break;
                    }
                case AbcTraitKind.Class:
                case AbcTraitKind.Function:
                case AbcTraitKind.Method:
                case AbcTraitKind.Getter:
                case AbcTraitKind.Setter:
                    {
                        int slotId = bytes.U30();
                        trait = new AbcTrait(name, (AbcTraitKind)kind, slotId, null, bytes.U30());
                        break;
                    }
                default:
                    throw new AbcFormatException($"Trait kind {kind} is not an ABC trait kind.");
            }
            if ((attributes & TraitAttributeMetadata) != 0)
            {
                int count = bytes.U30();
                for (int i = 0; i < count; i++)
                    bytes.U30();
            }
            return trait;
        }

        private static AbcMethodBody ReadBody(AbcBytes bytes)
        {
            int method = bytes.U30();
            bytes.U30();
            int localCount = bytes.U30();
            bytes.U30();
            bytes.U30();
            byte[] code = bytes.Bytes(bytes.U30());
            var exceptions = Repeat(bytes, () =>
            {
                var handler = new AbcExceptionHandler(bytes.U30(), bytes.U30(), bytes.U30());
                bytes.U30();
                bytes.U30();
                return handler;
            });
            return new AbcMethodBody(method, localCount, code, exceptions, ReadTraits(bytes));
        }
    }

    /// <summary>The variable-length integers and fixed values of ABC, over a byte span; reads past the end throw <see cref="AbcFormatException"/>.</summary>
    internal sealed class AbcBytes(byte[] data, int start, int length)
    {
        private readonly int _end = start + length;

        public int Position { get; private set; } = start;

        public bool AtEnd => Position >= _end;

        /// <summary>Throws unless <paramref name="count"/> entries of at least one byte each can still follow, so a corrupt count never sizes an allocation.</summary>
        public void RequireEntries(int count)
        {
            if (count > _end - Position)
                throw new AbcFormatException($"A list at {Position} claims {count} entries but only {_end - Position} bytes are left.");
        }

        public byte U8()
        {
            Require(1);
            return data[Position++];
        }

        public sbyte S8() => unchecked((sbyte)U8());

        public int U16()
        {
            Require(2);
            int value = data[Position] | (data[Position + 1] << 8);
            Position += 2;
            return value;
        }

        public int S24()
        {
            Require(3);
            int value = data[Position] | (data[Position + 1] << 8) | (data[Position + 2] << 16);
            Position += 3;
            return (value & 0x800000) != 0 ? value - 0x1000000 : value;
        }

        public uint U32()
        {
            uint result = 0;
            for (int i = 0; i < 5; i++)
            {
                byte b = U8();
                result |= (uint)(b & 0x7F) << (7 * i);
                if ((b & 0x80) == 0)
                    return result;
            }
            throw new AbcFormatException($"A variable-length integer at {Position} runs past five bytes.");
        }

        public int U30()
        {
            uint value = U32();
            return value <= 0x3FFFFFFF ? (int)value : throw new AbcFormatException($"The value {value} does not fit the 30 bits of a u30.");
        }

        public int S32()
        {
            uint result = 0;
            int shift = 0;
            for (int i = 0; i < 5; i++)
            {
                byte b = U8();
                result |= (uint)(b & 0x7F) << shift;
                shift += 7;
                if ((b & 0x80) == 0)
                {
                    if (shift < 32 && (b & 0x40) != 0)
                        result |= uint.MaxValue << shift;
                    break;
                }
            }
            return unchecked((int)result);
        }

        public double Double()
        {
            Require(8);
            double value = BitConverter.ToDouble(data, Position);
            Position += 8;
            return value;
        }

        public string Utf8()
        {
            int size = U30();
            Require(size);
            string value = Encoding.UTF8.GetString(data, Position, size);
            Position += size;
            return value;
        }

        public byte[] Bytes(int count)
        {
            Require(count);
            byte[] value = data.AsSpan(Position, count).ToArray();
            Position += count;
            return value;
        }

        private void Require(int count)
        {
            if (count < 0 || Position + count > _end)
                throw new AbcFormatException($"The bytecode ends while reading at {Position} ({_end - Position} bytes left, {count} needed).");
        }
    }
}
