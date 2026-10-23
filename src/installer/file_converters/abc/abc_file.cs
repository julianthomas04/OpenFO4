namespace AssetConverter.Abc
{
    internal sealed class AbcFormatException(string message) : Exception(message);

    internal enum AbcNamespaceKind
    {
        Namespace = 0x08,
        PackageNamespace = 0x16,
        PackageInternalNs = 0x17,
        ProtectedNamespace = 0x18,
        ExplicitNamespace = 0x19,
        StaticProtectedNs = 0x1A,
        PrivateNs = 0x05,
    }

    internal sealed record AbcNamespace(AbcNamespaceKind Kind, string Name);

    internal enum AbcMultinameKind
    {
        QName = 0x07,
        QNameA = 0x0D,
        RTQName = 0x0F,
        RTQNameA = 0x10,
        RTQNameL = 0x11,
        RTQNameLA = 0x12,
        Multiname = 0x09,
        MultinameA = 0x0E,
        MultinameL = 0x1B,
        MultinameLA = 0x1C,
        TypeName = 0x1D,
    }

    /// <summary>
    /// A multiname: its kind, namespace (QName) and local name; a plain Multiname carries the set of namespaces it is looked up in; a type
    /// name carries the qualified base and its parameters.
    /// </summary>
    internal sealed record AbcMultiname(
        AbcMultinameKind Kind, AbcNamespace? Namespace, string? Name, int TypeBase = 0, IReadOnlyList<int>? TypeParameters = null,
        IReadOnlyList<AbcNamespace>? NamespaceSet = null)
    {
        /// <summary>How many values the multiname takes from the operand stack at run time (namespace and/or name).</summary>
        public int RuntimeOperands => Kind switch
        {
            AbcMultinameKind.RTQName or AbcMultinameKind.RTQNameA or AbcMultinameKind.MultinameL or AbcMultinameKind.MultinameLA => 1,
            AbcMultinameKind.RTQNameL or AbcMultinameKind.RTQNameLA => 2,
            _ => 0,
        };

        public bool HasRuntimeName => Kind is AbcMultinameKind.RTQNameL or AbcMultinameKind.RTQNameLA or AbcMultinameKind.MultinameL or AbcMultinameKind.MultinameLA;
    }

    [Flags]
    internal enum AbcMethodFlags
    {
        None = 0,
        NeedArguments = 0x01,
        NeedActivation = 0x02,
        NeedRest = 0x04,
        HasOptional = 0x08,
        SetDxns = 0x40,
        HasParamNames = 0x80,
    }

    internal sealed record AbcMethod(int ParameterCount, string Name, AbcMethodFlags Flags);

    internal enum AbcTraitKind
    {
        Slot = 0,
        Method = 1,
        Getter = 2,
        Setter = 3,
        Class = 4,
        Function = 5,
        Const = 6,
    }

    /// <summary>A constant-pool value: the kind byte of a slot default or optional parameter and the index into its pool.</summary>
    internal readonly record struct AbcValueRef(int Kind, int Index);

    /// <param name="Default">The slot's default value; <c>null</c> when the slot states none (vindex 0).</param>
    /// <param name="Target">Method index for methods/getters/setters/functions, class index for class traits.</param>
    internal sealed record AbcTrait(int Name, AbcTraitKind Kind, int SlotId, AbcValueRef? Default, int Target);

    internal sealed record AbcInstance(int Name, int SuperName, int Flags, int Initializer, IReadOnlyList<AbcTrait> Traits);

    internal sealed record AbcClass(int Initializer, IReadOnlyList<AbcTrait> Traits);

    internal sealed record AbcScript(int Initializer, IReadOnlyList<AbcTrait> Traits);

    internal sealed record AbcExceptionHandler(int From, int To, int Target);

    internal sealed record AbcMethodBody(int Method, int LocalCount, byte[] Code, IReadOnlyList<AbcExceptionHandler> Exceptions, IReadOnlyList<AbcTrait> Traits);

    /// <summary>One parsed ABC (AVM2 bytecode) block. Pool index 0 of the number and string pools is the implicit default entry.</summary>
    internal sealed record AbcFile(
        IReadOnlyList<int> Ints,
        IReadOnlyList<uint> UInts,
        IReadOnlyList<double> Doubles,
        IReadOnlyList<string?> Strings,
        IReadOnlyList<AbcNamespace?> Namespaces,
        IReadOnlyList<AbcMultiname?> Multinames,
        IReadOnlyList<AbcMethod> Methods,
        IReadOnlyList<AbcInstance> Instances,
        IReadOnlyList<AbcClass> Classes,
        IReadOnlyList<AbcScript> Scripts,
        IReadOnlyList<AbcMethodBody> Bodies)
    {
        public AbcMultiname Multiname(int index) =>
            index > 0 && index < Multinames.Count && Multinames[index] is { } name
                ? name
                : throw new AbcFormatException($"Multiname index {index} is outside the pool of {Multinames.Count}.");

        public string String(int index) =>
            index > 0 && index < Strings.Count && Strings[index] is { } text
                ? text
                : throw new AbcFormatException($"String index {index} is outside the pool of {Strings.Count}.");

        /// <summary>
        /// The dotted qualified name of a QName: <c>package.Name</c>, or the bare name in the empty package. A plain Multiname names the class
        /// of this file that is defined in one of its namespaces; a class the file does not define keeps its bare name.
        /// </summary>
        public string QualifiedName(int multiname)
        {
            AbcMultiname name = Multiname(multiname);
            string local = name.Name ?? throw new AbcFormatException($"Multiname {multiname} has no name.");
            AbcNamespace? ns = name.Namespace ?? DefiningNamespace(name, local);
            string package = ns?.Name ?? "";
            return package.Length == 0 || ns?.Kind == AbcNamespaceKind.PrivateNs ? local : $"{package}.{local}";
        }

        private AbcNamespace? DefiningNamespace(AbcMultiname name, string local)
        {
            if (name.NamespaceSet is not { } lookedUp)
                return null;
            var defining = Instances.Select(instance => Multiname(instance.Name))
                .Where(defined => defined.Name == local && defined.Namespace is { } ns && lookedUp.Contains(ns))
                .Select(defined => defined.Namespace!).Distinct().ToList();
            return defining.Count == 1 ? defining[0] : null;
        }

        /// <summary>The local name of a multiname used as a property name, or the type name with its parameters for a TypeName.</summary>
        public string LocalName(int multiname)
        {
            AbcMultiname name = Multiname(multiname);
            if (name.Kind == AbcMultinameKind.TypeName)
                return $"{LocalName(name.TypeBase)}.<{string.Join(",", (name.TypeParameters ?? []).Select(p => p == 0 ? "*" : LocalName(p)))}>";
            return name.Name ?? "*";
        }
    }
}
