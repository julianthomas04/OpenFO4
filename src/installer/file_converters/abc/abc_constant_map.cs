namespace AssetConverter.Abc
{
    /// <summary>The constants of one method signature, in bytecode order, and which of them (by position) were folded from constant operands.</summary>
    internal sealed record AbcLiteralList(IReadOnlyList<object> Values, IReadOnlyList<int> Folded);

    /// <summary>
    /// The addressable values of a movie's ActionScript:
    /// <list type="bullet">
    /// <item><c>Constants</c>, keyed <c>Class.member</c>: a slot's stated default or the one unconditional constant its initialiser assigns.</item>
    /// <item><c>Literals</c>, keyed <c>Class.method#signature</c>: every constant inside a method body, by the signature of its consumer.</item>
    /// <item><c>Unresolved</c>: keys that exist but have no single value, with the reason (assigned twice, computed, conditional, a
    /// signature with an operand that cannot be described, ...); a bare class name when its definitions extend different classes.</item>
    /// <item><c>SuperClasses</c>: the class each defined class extends, by qualified name, where every definition agrees.</item>
    /// <item><c>ReferenceLists</c>, keyed <c>Class.member</c>: a member the initialiser assigns an array literal of looked-up names (the
    /// names the list holds, qualified where they are classes, in order); such a member is in no other map.</item>
    /// </list>
    /// </summary>
    internal sealed record AbcConstantMap(
        IReadOnlyDictionary<string, object> Constants,
        IReadOnlySet<string> FoldedConstants,
        IReadOnlyDictionary<string, AbcLiteralList> Literals,
        IReadOnlyDictionary<string, string> Unresolved,
        IReadOnlyDictionary<string, string> SuperClasses,
        IReadOnlyDictionary<string, IReadOnlyList<string>> ReferenceLists);

    internal static class AbcConstantExtractor
    {
        public const string Initializer = "<init>";
        public const string ClassInitializer = "<cinit>";

        private const int ValueUtf8 = 0x01;
        private const int ValueInt = 0x03;
        private const int ValueUInt = 0x04;
        private const int ValueDouble = 0x06;
        private const int ValueFalse = 0x0A;
        private const int ValueTrue = 0x0B;
        private const int ValueNull = 0x0C;
        private const int ValueUndefined = 0x00;

        /// <summary>The scripts of every ABC block a movie carries.</summary>
        public static AbcConstantMap Extract(Swf.SwfMovie movie, string movieName) =>
            Extract(movie.AbcBlocks.Select(block => (block.Name, AbcReader.Read(block.Data))).ToList(), movieName);

        public static AbcConstantMap Extract(IReadOnlyList<(string Name, AbcFile File)> blocks, string movie)
        {
            var perBlock = blocks.Select(block => ExtractBlock(block.File, $"{movie} ABC block '{block.Name}'")).ToList();
            return MergeBlocks(perBlock);
        }

        private sealed class BlockResult
        {
            public Dictionary<string, object> Constants { get; } = new(StringComparer.Ordinal);
            public HashSet<string> Folded { get; } = new(StringComparer.Ordinal);
            public Dictionary<string, AbcLiteralList> Literals { get; } = new(StringComparer.Ordinal);
            public Dictionary<string, string> Unresolved { get; } = new(StringComparer.Ordinal);
            public Dictionary<string, string> SuperClasses { get; } = new(StringComparer.Ordinal);
            public Dictionary<string, IReadOnlyList<string>> ReferenceLists { get; } = new(StringComparer.Ordinal);

            public IEnumerable<string> Keys => Constants.Keys.Concat(Literals.Keys).Concat(Unresolved.Keys).Concat(ReferenceLists.Keys);
        }

        private static AbcConstantMap MergeBlocks(List<BlockResult> blocks)
        {
            var counts = blocks.SelectMany(b => b.Keys.Distinct()).GroupBy(k => k, StringComparer.Ordinal).ToDictionary(g => g.Key, g => g.Count(), StringComparer.Ordinal);
            var constants = new SortedDictionary<string, object>(StringComparer.Ordinal);
            var folded = new SortedSet<string>(StringComparer.Ordinal);
            var literals = new SortedDictionary<string, AbcLiteralList>(StringComparer.Ordinal);
            var unresolved = new SortedDictionary<string, string>(StringComparer.Ordinal);
            var superClasses = new SortedDictionary<string, string>(StringComparer.Ordinal);
            var referenceLists = new SortedDictionary<string, IReadOnlyList<string>>(StringComparer.Ordinal);
            foreach (var definitions in blocks.SelectMany(b => b.SuperClasses).GroupBy(p => p.Key, StringComparer.Ordinal))
            {
                int distinct = definitions.Select(p => p.Value).Distinct(StringComparer.Ordinal).Count();
                if (distinct == 1)
                    superClasses[definitions.Key] = definitions.First().Value;
                else
                    unresolved[definitions.Key] = $"extends {distinct} different classes";
            }
            foreach (BlockResult block in blocks)
            {
                foreach (string key in block.Keys.Where(k => counts[k] > 1 && !unresolved.ContainsKey(k)))
                    unresolved[key] = $"defined in {counts[key]} ABC blocks";
                foreach ((string key, object value) in block.Constants.Where(p => counts[p.Key] == 1))
                    constants[key] = value;
                folded.UnionWith(block.Folded.Where(k => counts[k] == 1));
                foreach ((string key, AbcLiteralList list) in block.Literals.Where(p => counts[p.Key] == 1))
                    literals[key] = list;
                foreach ((string key, string reason) in block.Unresolved.Where(p => counts[p.Key] == 1))
                    unresolved[key] = reason;
                foreach ((string key, IReadOnlyList<string> names) in block.ReferenceLists.Where(p => counts[p.Key] == 1))
                    referenceLists[key] = names;
            }
            foreach (string key in unresolved.Keys)
            {
                constants.Remove(key);
                folded.Remove(key);
                literals.Remove(key);
                referenceLists.Remove(key);
            }
            return new AbcConstantMap(constants, folded, literals, unresolved, superClasses, referenceLists);
        }

        private static BlockResult ExtractBlock(AbcFile abc, string where)
        {
            var result = new BlockResult();
            Dictionary<int, AbcMethodBody> bodies = abc.Bodies.GroupBy(b => b.Method).ToDictionary(
                g => g.Key, g => g.Count() == 1 ? g.First() : throw new AbcFormatException($"{where}: method {g.Key} has {g.Count()} bodies."));
            List<(int Method, string Name)> names = NamedMethods(abc);
            var analysed = new Dictionary<int, AbcMethodLiterals>();
            var methodNames = new Dictionary<string, List<int>>(StringComparer.Ordinal);
            var queue = new Queue<(int Method, string Name)>(names);
            var seen = new HashSet<int>();
            while (queue.Count > 0)
            {
                (int method, string name) = queue.Dequeue();
                if (!methodNames.TryGetValue(name, out List<int>? owners))
                    methodNames[name] = owners = [];
                owners.Add(method);
                if (!seen.Add(method) || !bodies.TryGetValue(method, out AbcMethodBody? body))
                    continue;
                AbcMethodLiterals literals = AbcLiteralFlow.Analyze(abc, body, $"{where}, {name}");
                analysed[method] = literals;
                for (int k = 0; k < literals.Closures.Count; k++)
                    queue.Enqueue((literals.Closures[k], $"{name}/fn{k}"));
            }
            foreach (AbcMethodBody body in abc.Bodies.Where(b => !seen.Contains(b.Method)))
            {
                string name = $"<method{body.Method}>";
                methodNames[name] = [body.Method];
                analysed[body.Method] = AbcLiteralFlow.Analyze(abc, body, $"{where}, {name}");
            }

            foreach ((string name, List<int> methods) in methodNames)
            {
                var distinct = methods.Distinct().Where(analysed.ContainsKey).ToList();
                if (distinct.Count == 0)
                    continue;
                if (distinct.Count > 1)
                {
                    foreach (string key in distinct.SelectMany(m => LiteralKeys(name, analysed[m])).Distinct())
                        result.Unresolved[Displayed(key)] = $"the method name '{name}' names {distinct.Count} methods";
                    continue;
                }
                foreach ((string key, AbcLiteralList list) in LiteralLists(name, analysed[distinct[0]]))
                {
                    if (key.Contains(AbcValues.Undescribable, StringComparison.Ordinal))
                        result.Unresolved[Displayed(key)] = OperandNotDescribed;
                    else
                        result.Literals[key] = list;
                }
            }

            ResolveMembers(abc, analysed, result);
            var conflicting = new HashSet<string>(StringComparer.Ordinal);
            foreach (AbcInstance instance in abc.Instances.Where(i => i.SuperName != 0))
            {
                string className = abc.QualifiedName(instance.Name);
                string superClass = abc.QualifiedName(instance.SuperName);
                if (result.SuperClasses.TryGetValue(className, out string? stated) && stated != superClass)
                    conflicting.Add(className);
                result.SuperClasses[className] = superClass;
            }
            foreach (string className in conflicting)
            {
                result.SuperClasses.Remove(className);
                result.Unresolved[className] = "extends 2 different classes";
            }
            return result;
        }

        private const string OperandNotDescribed = "operand cannot be described";

        /// <summary>A key as written to a document, with an operand that cannot be described shown as <c>?</c>.</summary>
        private static string Displayed(string key) => key.Replace(AbcValues.Undescribable, '?');

        private static IEnumerable<string> LiteralKeys(string method, AbcMethodLiterals literals) =>
            literals.Uses.Select(u => $"{method}#{u.Signature}");

        private static IEnumerable<(string, AbcLiteralList)> LiteralLists(string method, AbcMethodLiterals literals) =>
            literals.Uses.GroupBy(u => u.Signature, StringComparer.Ordinal).Select(g =>
            {
                var uses = g.OrderBy(u => u.Literal.Origin).ToList();
                return ($"{method}#{g.Key}", new AbcLiteralList(
                    uses.Select(u => u.Literal.Value).ToList(),
                    uses.Select((u, i) => (u, i)).Where(p => p.u.Literal.Folded).Select(p => p.i).ToList()));
            });

        /// <summary>Every method a trait or initialiser names, as <c>Class.method</c> (<c>get:</c>/<c>set:</c> for accessors).</summary>
        private static List<(int, string)> NamedMethods(AbcFile abc)
        {
            var names = new List<(int, string)>();
            for (int i = 0; i < abc.Instances.Count; i++)
            {
                string className = abc.QualifiedName(abc.Instances[i].Name);
                names.Add((abc.Instances[i].Initializer, $"{className}.{Initializer}"));
                names.Add((abc.Classes[i].Initializer, $"{className}.{ClassInitializer}"));
                names.AddRange(MethodTraits(abc, abc.Instances[i].Traits, className + "."));
                names.AddRange(MethodTraits(abc, abc.Classes[i].Traits, className + "."));
            }
            for (int s = 0; s < abc.Scripts.Count; s++)
            {
                names.Add((abc.Scripts[s].Initializer, $"<script{s}>"));
                names.AddRange(abc.Scripts[s].Traits
                    .Where(t => t.Kind is AbcTraitKind.Method or AbcTraitKind.Getter or AbcTraitKind.Setter or AbcTraitKind.Function)
                    .Select(t => (t.Target, AccessorPrefix(t.Kind) + abc.QualifiedName(t.Name))));
            }
            return names;
        }

        private static IEnumerable<(int, string)> MethodTraits(AbcFile abc, IReadOnlyList<AbcTrait> traits, string owner) =>
            traits.Where(t => t.Kind is AbcTraitKind.Method or AbcTraitKind.Getter or AbcTraitKind.Setter or AbcTraitKind.Function)
                .Select(t => (t.Target, owner + AccessorPrefix(t.Kind) + abc.LocalName(t.Name)));

        private static string AccessorPrefix(AbcTraitKind kind) => kind switch
        {
            AbcTraitKind.Getter => "get:",
            AbcTraitKind.Setter => "set:",
            _ => "",
        };

        private static void ResolveMembers(AbcFile abc, Dictionary<int, AbcMethodLiterals> analysed, BlockResult result)
        {
            for (int i = 0; i < abc.Instances.Count; i++)
            {
                string className = abc.QualifiedName(abc.Instances[i].Name);
                var instances = new BlockResult();
                var statics = new BlockResult();
                ResolveScope(abc, abc.Instances[i].Traits, analysed.GetValueOrDefault(abc.Instances[i].Initializer), className + ".", instances);
                ResolveScope(abc, abc.Classes[i].Traits, analysed.GetValueOrDefault(abc.Classes[i].Initializer), className + ".", statics);
                var shared = instances.Keys.Intersect(statics.Keys, StringComparer.Ordinal).ToHashSet(StringComparer.Ordinal);
                foreach (BlockResult scope in new[] { instances, statics })
                {
                    foreach ((string key, object value) in scope.Constants.Where(p => !shared.Contains(p.Key)))
                        result.Constants[key] = value;
                    result.Folded.UnionWith(scope.Folded.Where(k => !shared.Contains(k)));
                    foreach ((string key, string reason) in scope.Unresolved.Where(p => !shared.Contains(p.Key)))
                        result.Unresolved[key] = reason;
                    foreach ((string key, IReadOnlyList<string> names) in scope.ReferenceLists.Where(p => !shared.Contains(p.Key)))
                        result.ReferenceLists[key] = names;
                }
                foreach (string key in shared)
                    result.Unresolved[key] = "set by both the class and its instance initialiser";
            }
            foreach (AbcScript script in abc.Scripts)
                ResolveScope(abc, script.Traits, analysed.GetValueOrDefault(script.Initializer), null, result);
        }

        /// <param name="owner">The class prefix, or <c>null</c> for package-level members, whose keys are their qualified names.</param>
        private static void ResolveScope(AbcFile abc, IReadOnlyList<AbcTrait> traits, AbcMethodLiterals? initializer, string? owner, BlockResult result)
        {
            var assignments = (initializer?.Assignments ?? []).GroupBy(a => a.Member, StringComparer.Ordinal).ToDictionary(g => g.Key, g => g.ToList(), StringComparer.Ordinal);
            var slots = traits.Where(t => t.Kind is AbcTraitKind.Slot or AbcTraitKind.Const).ToList();
            var members = new Dictionary<string, (string Key, AbcTrait? Trait)>(StringComparer.Ordinal);
            foreach (AbcTrait slot in slots)
                members[abc.LocalName(slot.Name)] = (owner is null ? abc.QualifiedName(slot.Name) : owner + abc.LocalName(slot.Name), slot);
            if (owner is not null)
                foreach (string member in assignments.Keys.Where(m => !members.ContainsKey(m)))
                    members[member] = (owner + member, null);

            foreach ((string member, (string key, AbcTrait? trait)) in members)
            {
                var sources = new List<AbcMemberAssignment>(assignments.GetValueOrDefault(member) ?? []);
                if (trait?.Default is { } stated)
                {
                    if (!TryStatedValue(abc, stated, out object? value, out bool isEmpty))
                    {
                        result.Unresolved[key] = "its stated default is not a plain value";
                        continue;
                    }
                    if (!isEmpty)
                        sources.Insert(0, new AbcMemberAssignment(member, new AbcLiteral(-1, value!, false), true));
                }
                Resolve(key, sources, result);
            }
        }

        /// <summary>
        /// One value or none: a slot's stated default and an unconditional assignment of the same constant (compilers emit both for a
        /// constant initialiser) count as one; differing values, a computed value or a conditional assignment leave the key unresolved.
        /// </summary>
        private static void Resolve(string key, List<AbcMemberAssignment> sources, BlockResult result)
        {
            if (sources.Count == 0)
            {
                result.Unresolved[key] = "declared without a value";
                return;
            }
            if (sources is [{ References: { } names }])
            {
                if (sources[0].StraightLine)
                    result.ReferenceLists[key] = names;
                else
                    result.Unresolved[key] = "assigned conditionally";
                return;
            }
            if (sources.Any(s => s.Literal is null))
            {
                result.Unresolved[key] = "computed from an expression";
                return;
            }
            if (sources.Count > 1 && !(sources.All(s => s.StraightLine) && sources.Select(s => AbcValues.Describe(AbcValues.IsNumber(s.Literal!.Value) ? AbcValues.ToNumber(s.Literal.Value) : s.Literal.Value)).Distinct().Count() == 1))
            {
                result.Unresolved[key] = $"assigned {sources.Count} times";
                return;
            }
            if (!sources[0].StraightLine)
            {
                result.Unresolved[key] = "assigned conditionally";
                return;
            }
            result.Constants[key] = sources[0].Literal!.Value;
            if (sources[0].Literal!.Folded)
                result.Folded.Add(key);
        }

        /// <param name="isEmpty">Set for a stated null or undefined, which is the same as stating nothing.</param>
        private static bool TryStatedValue(AbcFile abc, AbcValueRef stated, out object? value, out bool isEmpty)
        {
            isEmpty = false;
            value = stated.Kind switch
            {
                ValueInt => Pool(abc.Ints, stated.Index),
                ValueUInt => Pool(abc.UInts, stated.Index),
                ValueDouble => Pool(abc.Doubles, stated.Index) is var d && double.IsFinite(d) ? d : null,
                ValueUtf8 => abc.String(stated.Index),
                ValueTrue => true,
                ValueFalse => false,
                _ => null,
            };
            if (stated.Kind is ValueNull or ValueUndefined)
            {
                isEmpty = true;
                return true;
            }
            return value is not null;
        }

        private static T Pool<T>(IReadOnlyList<T> pool, int index) =>
            index > 0 && index < pool.Count ? pool[index] : throw new AbcFormatException($"A slot default names pool index {index} outside the pool of {pool.Count}.");
    }
}
