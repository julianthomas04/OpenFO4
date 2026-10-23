using System.Text;

namespace InstallerTests
{
    /// <summary>A trait of a class, instance or script as the ABC builder writes it.</summary>
    internal abstract record AbcTraitSpec(int Name);

    /// <param name="ValueKind">The constant kind byte (0x03 int, 0x06 double, 0x01 string, ...); ignored when <paramref name="ValueIndex"/> is 0.</param>
    internal sealed record AbcSlotSpec(int Name, bool IsConst = false, int ValueIndex = 0, int ValueKind = 0) : AbcTraitSpec(Name);

    /// <param name="Kind">1 method, 2 getter, 3 setter.</param>
    internal sealed record AbcMethodTraitSpec(int Name, int Method, int Kind = 1) : AbcTraitSpec(Name);

    /// <summary>
    /// Builds ABC (AVM2 bytecode) blocks: constant pools with interning, methods, method bodies of raw bytecode, classes and scripts.
    /// Method bodies are written by <see cref="AbcCode"/>.
    /// </summary>
    internal sealed class AbcBuilder
    {
        public const int ConstantUtf8 = 0x01;
        public const int ConstantInt = 0x03;
        public const int ConstantDouble = 0x06;
        public const int ConstantTrue = 0x0B;
        public const int ConstantFalse = 0x0A;
        public const int PackageNamespace = 0x16;
        public const int PrivateNamespace = 0x05;
        public const int QName = 0x07;
        public const int MultinameL = 0x1B;

        private readonly List<int> _ints = [];
        private readonly List<uint> _uints = [];
        private readonly List<double> _doubles = [];
        private readonly List<string> _strings = [];
        private readonly List<(int Kind, int Name)> _namespaces = [];
        private readonly List<byte[]> _namespaceSets = [];
        private readonly List<byte[]> _multinames = [];
        private readonly List<(int Parameters, int Name)> _methods = [];
        private readonly List<byte[]> _bodies = [];
        private readonly List<(byte[] Instance, byte[] Class)> _classes = [];
        private readonly List<byte[]> _scripts = [];

        public int Int(int value) => Intern(_ints, value);

        public int UInt(uint value) => Intern(_uints, value);

        public int Double(double value) => Intern(_doubles, value);

        public int String(string value) => Intern(_strings, value);

        public int Namespace(string name, int kind = PackageNamespace) => Intern(_namespaces, (kind, String(name)));

        /// <summary>A QName in the package <paramref name="package"/> (empty for the public top level).</summary>
        public int Name(string local, string package = "", int namespaceKind = PackageNamespace)
        {
            byte[] entry = [QName, .. U30(Namespace(package, namespaceKind)), .. U30(String(local))];
            return Intern(_multinames, entry);
        }

        /// <summary>A late-bound name (<c>obj[expr]</c>) over a namespace set of the public namespace.</summary>
        public int RuntimeName()
        {
            int set = Intern(_namespaceSets, [.. U30(1), .. U30(Namespace(""))]);
            return Intern(_multinames, [MultinameL, .. U30(set)]);
        }

        public int Method(int parameters = 0, string name = "")
        {
            _methods.Add((parameters, name.Length == 0 ? 0 : String(name)));
            return _methods.Count - 1;
        }

        /// <param name="exceptions">Each handler's start, end and target code offsets.</param>
        public int Body(int method, byte[] code, int localCount = 1, (int From, int To, int Target)[]? exceptions = null)
        {
            var body = new List<byte>();
            body.AddRange(U30(method));
            body.AddRange(U30(8));
            body.AddRange(U30(localCount));
            body.AddRange(U30(0));
            body.AddRange(U30(8));
            body.AddRange(U30(code.Length));
            body.AddRange(code);
            body.AddRange(U30(exceptions?.Length ?? 0));
            foreach ((int from, int to, int target) in exceptions ?? [])
                body.AddRange([.. U30(from), .. U30(to), .. U30(target), .. U30(0), .. U30(0)]);
            body.AddRange(U30(0));
            _bodies.Add([.. body]);
            return method;
        }

        /// <summary>A method with a body, in one call.</summary>
        public int Function(byte[] code, int parameters = 0, int localCount = 1) => Body(Method(parameters), code, Math.Max(localCount, parameters + 1));

        /// <summary>A class <paramref name="package"/>.<paramref name="name"/> extending <paramref name="superName"/> (a multiname, 0 for none).</summary>
        public void Class(
            string package, string name, int superName, int instanceInit, int classInit,
            IEnumerable<AbcTraitSpec>? instanceTraits = null, IEnumerable<AbcTraitSpec>? classTraits = null)
        {
            byte[] instance = [.. U30(Name(name, package)), .. U30(superName), 0, .. U30(0), .. U30(instanceInit), .. Traits(instanceTraits)];
            byte[] cls = [.. U30(classInit), .. Traits(classTraits)];
            _classes.Add((instance, cls));
        }

        public void Script(int init, IEnumerable<AbcTraitSpec>? traits = null) => _scripts.Add([.. U30(init), .. Traits(traits)]);

        public byte[] Build()
        {
            var w = new List<byte>();
            w.AddRange(BitConverter.GetBytes((ushort)16));
            w.AddRange(BitConverter.GetBytes((ushort)46));
            WritePool(w, _ints, v => S32(v));
            WritePool(w, _uints, v => U32(v));
            WritePool(w, _doubles, BitConverter.GetBytes);
            WritePool(w, _strings, s => [.. U30(Encoding.UTF8.GetByteCount(s)), .. Encoding.UTF8.GetBytes(s)]);
            WritePool(w, _namespaces, ns => [(byte)ns.Kind, .. U30(ns.Name)]);
            WritePool(w, _namespaceSets, set => set);
            WritePool(w, _multinames, name => name);
            w.AddRange(U30(_methods.Count));
            foreach ((int parameters, int name) in _methods)
            {
                w.AddRange(U30(parameters));
                w.AddRange(U30(0));
                for (int i = 0; i < parameters; i++)
                    w.AddRange(U30(0));
                w.AddRange(U30(name));
                w.Add(0);
            }
            w.AddRange(U30(0));
            w.AddRange(U30(_classes.Count));
            foreach ((byte[] instance, _) in _classes)
                w.AddRange(instance);
            foreach ((_, byte[] cls) in _classes)
                w.AddRange(cls);
            w.AddRange(U30(_scripts.Count));
            foreach (byte[] script in _scripts)
                w.AddRange(script);
            w.AddRange(U30(_bodies.Count));
            foreach (byte[] body in _bodies)
                w.AddRange(body);
            return [.. w];
        }

        public static byte[] U30(int value) => U32(checked((uint)value));

        public static byte[] U32(uint value)
        {
            var bytes = new List<byte>();
            do
            {
                byte b = (byte)(value & 0x7F);
                value >>= 7;
                bytes.Add(value != 0 ? (byte)(b | 0x80) : b);
            }
            while (value != 0);
            return [.. bytes];
        }

        public static byte[] S32(int value)
        {
            var bytes = new List<byte>();
            while (true)
            {
                byte b = (byte)(value & 0x7F);
                value >>= 7;
                bool done = (value == 0 && (b & 0x40) == 0) || (value == -1 && (b & 0x40) != 0);
                bytes.Add(done ? b : (byte)(b | 0x80));
                if (done)
                    return [.. bytes];
            }
        }

        private static byte[] Traits(IEnumerable<AbcTraitSpec>? traits)
        {
            var list = traits?.ToList() ?? [];
            var w = new List<byte>(U30(list.Count));
            foreach (AbcTraitSpec trait in list)
            {
                w.AddRange(U30(trait.Name));
                switch (trait)
                {
                    case AbcSlotSpec slot:
                        w.Add((byte)(slot.IsConst ? 6 : 0));
                        w.AddRange([.. U30(0), .. U30(0), .. U30(slot.ValueIndex)]);
                        if (slot.ValueIndex != 0)
                            w.Add((byte)slot.ValueKind);
                        break;
                    case AbcMethodTraitSpec method:
                        w.Add((byte)method.Kind);
                        w.AddRange([.. U30(0), .. U30(method.Method)]);
                        break;
                }
            }
            return [.. w];
        }

        private static int Intern<T>(List<T> pool, T value)
        {
            int index = typeof(T) == typeof(byte[])
                ? pool.FindIndex(existing => ((byte[])(object)existing!).AsSpan().SequenceEqual((byte[])(object)value!))
                : pool.IndexOf(value);
            if (index >= 0)
                return index + 1;
            pool.Add(value);
            return pool.Count;
        }

        private static void WritePool<T>(List<byte> w, List<T> pool, Func<T, byte[]> write)
        {
            w.AddRange(U30(pool.Count == 0 ? 0 : pool.Count + 1));
            foreach (T value in pool)
                w.AddRange(write(value));
        }
    }

    /// <summary>Writes AVM2 bytecode for a method body; branch offsets are relative to the end of the branch instruction, as AVM2 states them.</summary>
    internal sealed class AbcCode
    {
        private readonly List<byte> _code = [];

        public int Offset => _code.Count;

        public AbcCode Op(byte opcode, params int[] u30Operands)
        {
            _code.Add(opcode);
            foreach (int operand in u30Operands)
                _code.AddRange(AbcBuilder.U30(operand));
            return this;
        }

        public AbcCode GetLocal0() => Op(0xD0);

        public AbcCode GetLocal(int index) => Op(0x62, index);

        public AbcCode PushScope() => Op(0x30);

        public AbcCode PushByte(sbyte value)
        {
            _code.Add(0x24);
            _code.Add(unchecked((byte)value));
            return this;
        }

        public AbcCode PushShort(short value)
        {
            _code.Add(0x25);
            _code.AddRange(AbcBuilder.U32(unchecked((uint)value)));
            return this;
        }

        public AbcCode PushInt(int index) => Op(0x2D, index);

        public AbcCode PushDouble(int index) => Op(0x2F, index);

        public AbcCode PushString(int index) => Op(0x2C, index);

        public AbcCode PushTrue() => Op(0x26);

        public AbcCode PushNull() => Op(0x20);

        public AbcCode Pop() => Op(0x29);

        public AbcCode GetLex(int name) => Op(0x60, name);

        public AbcCode GetProperty(int name) => Op(0x66, name);

        public AbcCode SetProperty(int name) => Op(0x61, name);

        public AbcCode InitProperty(int name) => Op(0x68, name);

        public AbcCode FindPropStrict(int name) => Op(0x5D, name);

        public AbcCode FindProperty(int name) => Op(0x5E, name);

        public AbcCode CallPropVoid(int name, int argumentCount) => Op(0x4F, name, argumentCount);

        public AbcCode CallProperty(int name, int argumentCount) => Op(0x46, name, argumentCount);

        public AbcCode ConstructSuper(int argumentCount) => Op(0x49, argumentCount);

        public AbcCode NewFunction(int method) => Op(0x40, method);

        public AbcCode Add() => Op(0xA0);

        public AbcCode Subtract() => Op(0xA1);

        public AbcCode Multiply() => Op(0xA2);

        public AbcCode Negate() => Op(0x90);

        public AbcCode ConvertDouble() => Op(0x75);

        public AbcCode ReturnVoid() => Op(0x47);

        public AbcCode Throw() => Op(0x03);

        public AbcCode Label() => Op(0x09);

        /// <summary>A branch whose target is patched by <see cref="Here"/>; returns the patch handle.</summary>
        public int Branch(byte opcode)
        {
            _code.Add(opcode);
            int handle = _code.Count;
            _code.AddRange([0, 0, 0]);
            return handle;
        }

        public int Jump() => Branch(0x10);

        public int IfTrue() => Branch(0x11);

        public int IfFalse() => Branch(0x12);

        /// <summary>Points the branch <paramref name="handle"/> at the current offset.</summary>
        public AbcCode Here(int handle)
        {
            int relative = _code.Count - (handle + 3);
            _code[handle] = (byte)relative;
            _code[handle + 1] = (byte)(relative >> 8);
            _code[handle + 2] = (byte)(relative >> 16);
            return this;
        }

        /// <summary>A branch back to <paramref name="target"/>.</summary>
        public AbcCode JumpTo(int target)
        {
            int handle = Jump();
            int relative = target - (handle + 3);
            _code[handle] = (byte)relative;
            _code[handle + 1] = (byte)(relative >> 8);
            _code[handle + 2] = (byte)(relative >> 16);
            return this;
        }

        public AbcCode Raw(params byte[] bytes)
        {
            _code.AddRange(bytes);
            return this;
        }

        public byte[] ToArray() => [.. _code];
    }
}
