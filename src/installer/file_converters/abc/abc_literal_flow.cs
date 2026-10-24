using System.Collections.Immutable;

namespace AssetConverter.Abc
{
    /// <summary>A constant the bytecode pushes or folds: the offset of the instruction that produced it, its value and whether it was folded.</summary>
    internal sealed record AbcLiteral(int Origin, object Value, bool Folded);

    /// <summary>A constant and the consumer signature it flows into, e.g. <c>List_mc.numListItems=</c> or <c>gotoAndPlay(0)</c>.</summary>
    internal sealed record AbcLiteralUse(string Signature, AbcLiteral Literal);

    /// <summary>
    /// An assignment to a member of the method's own object or scope (<c>this.x = ...</c>, <c>x = ...</c>): the constant assigned when the
    /// value is exactly one constant, and whether the assignment runs unconditionally once the method starts.
    /// </summary>
    /// <param name="References">The qualified names an array literal lists when every element is a looked-up name (a list of references); otherwise <c>null</c>.</param>
    internal sealed record AbcMemberAssignment(string Member, AbcLiteral? Literal, bool StraightLine, IReadOnlyList<string>? References = null);

    /// <summary>An argument of a call: its description, and its constant when the argument is exactly one constant.</summary>
    internal sealed record AbcCallArgument(string Description, object? Literal);

    /// <summary>
    /// A call of a method of the method's own object or scope (<c>stop()</c>, <c>this.gotoAndPlay(5)</c>), its arguments, and whether
    /// it runs whenever the method returns.
    /// </summary>
    internal sealed record AbcOwnCall(string Method, IReadOnlyList<AbcCallArgument> Arguments, bool Unconditional);

    /// <param name="Closures">The method indices of the functions the method creates, in bytecode order.</param>
    /// <param name="OwnCalls">The calls of the method's own object or scope, in bytecode order.</param>
    internal sealed record AbcMethodLiterals(
        IReadOnlyList<AbcLiteralUse> Uses, IReadOnlyList<AbcMemberAssignment> Assignments, IReadOnlyList<int> Closures, IReadOnlyList<AbcOwnCall> OwnCalls);

    /// <summary>
    /// Follows every constant of a method body through the operand stack to the instruction that consumes it, over all control-flow paths
    /// (a ternary's two constants both reach the assignment after it), and names that consumer by a short signature. Operators whose operands
    /// are all constants are folded with ActionScript semantics. A constant no consumer takes is reported under the signature <c>?</c>.
    /// An operand that cannot be described (an unmodelled instruction's result, paths that disagree, an over-long expression) appears in a
    /// signature as <see cref="AbcValues.Undescribable"/>, so its keys can be told apart from exact ones.
    /// </summary>
    internal static class AbcLiteralFlow
    {
        public const string Unconsumed = "?";
        private const string Unknown = "\uE000";
        private const string This = "this";
        private const string Scope = "";
        private const int MaxDescriptionLength = 1000;
        private const int MaxVisitsPerBlock = 64;

        public static AbcMethodLiterals Analyze(AbcFile abc, AbcMethodBody body, string where)
        {
            IReadOnlyList<AbcInstruction> code = AbcDecoder.Decode(body.Code, where);
            var analysis = new Analysis(abc, body, code, where);
            return analysis.Run();
        }

        private sealed class FlowValue : IEquatable<FlowValue>
        {
            public static readonly FlowValue Other = new(ImmutableSortedSet<int>.Empty, true, Unknown, null, false, null);

            public FlowValue(ImmutableSortedSet<int> origins, bool maybeOther, string description, object? literal, bool folded, ImmutableArray<(string Suffix, FlowValue Value)>? parts, string? reference = null)
            {
                Reference = reference;
                Origins = origins;
                MaybeOther = maybeOther;
                Description = description.Length > MaxDescriptionLength ? Unknown : description;
                Literal = literal;
                Folded = folded;
                Parts = parts;
            }

            public ImmutableSortedSet<int> Origins { get; }
            public bool MaybeOther { get; }
            public string Description { get; }

            /// <summary>Set when the value is exactly one constant on every path.</summary>
            public object? Literal { get; }
            public bool Folded { get; }
            public ImmutableArray<(string Suffix, FlowValue Value)>? Parts { get; }

            /// <summary>The qualified name when the value is exactly one looked-up name (<c>getlex</c>) on every path.</summary>
            public string? Reference { get; }

            public bool IsPure => Literal is not null;

            public static FlowValue Described(string description) => new(ImmutableSortedSet<int>.Empty, true, description, null, false, null);

            public static FlowValue Referenced(string description, string qualifiedName) =>
                new(ImmutableSortedSet<int>.Empty, true, description, null, false, null, qualifiedName);

            public static FlowValue Constant(int origin, object value, bool folded) =>
                new(ImmutableSortedSet.Create(origin), false, AbcValues.Describe(value), value, folded, null);

            public static FlowValue Composite(string description, ImmutableArray<(string, FlowValue)> parts) =>
                new(ImmutableSortedSet<int>.Empty, true, description, null, false, parts);

            public FlowValue Merge(FlowValue other)
            {
                if (Equals(other))
                    return this;
                var origins = Origins.Union(other.Origins);
                bool maybeOther = MaybeOther || other.MaybeOther;
                string description = Description == other.Description ? Description : Unknown;
                bool samePure = IsPure && other.IsPure && Origins.SetEquals(other.Origins) && Equals(Literal, other.Literal);
                bool sameParts = Parts is { } mine && other.Parts is { } theirs && mine.SequenceEqual(theirs);
                return new FlowValue(origins, maybeOther, description, samePure ? Literal : null, samePure && Folded, sameParts ? Parts : null,
                    Reference == other.Reference ? Reference : null);
            }

            public bool Equals(FlowValue? other) =>
                other is not null && MaybeOther == other.MaybeOther && Description == other.Description && Equals(Literal, other.Literal)
                && Folded == other.Folded && Reference == other.Reference && Origins.SetEquals(other.Origins)
                && (Parts is { } mine ? other.Parts is { } theirs && mine.SequenceEqual(theirs) : other.Parts is null);

            public override bool Equals(object? obj) => Equals(obj as FlowValue);

            public override int GetHashCode() => HashCode.Combine(Description, Origins.Count, MaybeOther);
        }

        private sealed class Block(int start, int end)
        {
            public int Start { get; } = start;
            public int End { get; } = end;
            public List<int> Successors { get; } = [];
            public List<int> Predecessors { get; } = [];

            /// <summary>The stack control enters with from outside the flow: empty at the method's entry, the caught value at a handler.</summary>
            public ImmutableList<FlowValue>? Initial { get; set; }

            public ImmutableList<FlowValue>? Entry { get; set; }
            public ImmutableList<FlowValue>? Exit { get; set; }
            public int Visits { get; set; }
        }

        private sealed class Analysis(AbcFile abc, AbcMethodBody body, IReadOnlyList<AbcInstruction> code, string where)
        {
            private readonly Dictionary<int, int> _indexByOffset = code.Select((instruction, index) => (instruction.Offset, index)).ToDictionary(p => p.Offset, p => p.index);
            private readonly Dictionary<int, (object Value, bool Folded)> _literals = [];
            private readonly Dictionary<int, object> _originValues = [];
            private readonly HashSet<int> _internal = [];
            private readonly List<(int Origin, string Signature)> _uses = [];
            private readonly List<AbcMemberAssignment> _assignments = [];
            private readonly List<AbcOwnCall> _ownCalls = [];
            private readonly HashSet<int> _straight = [];
            private readonly Dictionary<int, (string Condition, bool TakenWhenTrue)> _conditions = [];
            private HashSet<int>[] _dominators = [];
            private HashSet<int> _inCycle = [];
            private bool _recording;

            public AbcMethodLiterals Run()
            {
                if (code.Count == 0)
                    return new AbcMethodLiterals([], [], [], []);
                List<Block> blocks = BuildBlocks();
                MarkUnconditional(blocks);
                _dominators = Dominators(blocks);
                _inCycle = BlocksInCycles(blocks);
                Solve(blocks);
                _literals.Clear();
                _recording = true;
                foreach (Block block in blocks.Where(b => b.Entry is not null))
                    Transfer(block, block.Entry!);
                return new AbcMethodLiterals(CollectUses(), _assignments, Closures(), _ownCalls);
            }

            private List<int> Closures() =>
                code.Where(i => i.Mnemonic == "newfunction").Select(i => i.Operands[0]).ToList();

            private List<AbcLiteralUse> CollectUses()
            {
                var consumed = _uses.Select(u => u.Origin).ToHashSet();
                var uses = _uses.Distinct().ToList();
                foreach (int origin in _literals.Keys.Where(o => !consumed.Contains(o) && !_internal.Contains(o)))
                    uses.Add((origin, Unconsumed));
                return uses.OrderBy(u => u.Origin).ThenBy(u => u.Signature, StringComparer.Ordinal)
                    .Select(u => new AbcLiteralUse(u.Signature, new AbcLiteral(u.Origin, _literals[u.Origin].Value, _literals[u.Origin].Folded)))
                    .ToList();
            }

            /// <summary>The instruction index at a branch target; the end of the code (a compiler's jump out of dead code) is index <c>code.Count</c>.</summary>
            private int IndexOf(int offset) =>
                _indexByOffset.TryGetValue(offset, out int index) ? index
                : offset == body.Code.Length ? code.Count
                : throw new AbcFormatException($"{where}: a branch targets offset {offset}, which starts no instruction.");

            private List<int> Successors(int index)
            {
                AbcInstruction instruction = code[index];
                var successors = instruction.SwitchTargets.Select(IndexOf).ToList();
                if (AbcOpcodes.IsBranch(instruction.Opcode))
                    successors.Add(IndexOf(instruction.BranchTarget));
                if (!AbcOpcodes.EndsFlow(instruction.Opcode))
                    successors.Add(index + 1);
                return successors.Where(s => s < code.Count).ToList();
            }

            /// <summary>
            /// The instructions control can reach from the entry and from the handlers guarding reachable code. Compilers leave dead code
            /// (a jump after a return) whose branch offsets may point anywhere; it is never decoded as flow.
            /// </summary>
            private (HashSet<int> Reachable, List<int> HandlerEntries) Reach()
            {
                var reachable = new HashSet<int>();
                var handlerEntries = new List<int>();
                var pending = new Stack<int>([0]);
                var handlers = body.Exceptions.ToList();
                List<AbcExceptionHandler> guarding;
                do
                {
                    while (pending.Count > 0)
                    {
                        int index = pending.Pop();
                        if (reachable.Add(index))
                            Successors(index).ForEach(pending.Push);
                    }
                    guarding = handlers.Where(h => reachable.Any(i => code[i].Offset >= h.From && code[i].Offset < h.To)).ToList();
                    foreach (AbcExceptionHandler handler in guarding)
                    {
                        handlers.Remove(handler);
                        int entry = IndexOf(handler.Target);
                        handlerEntries.Add(entry);
                        pending.Push(entry);
                    }
                }
                while (guarding.Count > 0);
                return (reachable, handlerEntries);
            }

            private List<Block> BuildBlocks()
            {
                (HashSet<int> reachable, List<int> handlerEntries) = Reach();
                var leaders = new SortedSet<int>(handlerEntries) { 0 };
                foreach (int i in reachable)
                {
                    AbcInstruction instruction = code[i];
                    if (AbcOpcodes.IsBranch(instruction.Opcode) || instruction.SwitchTargets.Count > 0 || AbcOpcodes.EndsFlow(instruction.Opcode))
                        leaders.UnionWith(Successors(i));
                    if (i + 1 < code.Count && !reachable.Contains(i + 1))
                        continue;
                    if (AbcOpcodes.IsBranch(instruction.Opcode) || AbcOpcodes.EndsFlow(instruction.Opcode))
                        leaders.Add(i + 1);
                }

                var blocks = new List<Block>();
                foreach (int start in leaders.Where(reachable.Contains))
                {
                    int end = start + 1;
                    while (end < code.Count && reachable.Contains(end) && !leaders.Contains(end))
                        end++;
                    blocks.Add(new Block(start, end));
                }
                var blockByStart = blocks.Select((b, k) => (b.Start, k)).ToDictionary(p => p.Start, p => p.k);
                foreach (Block block in blocks)
                    block.Successors.AddRange(Successors(block.End - 1).Select(t => blockByStart[t]));
                for (int k = 0; k < blocks.Count; k++)
                    foreach (int successor in blocks[k].Successors)
                        blocks[successor].Predecessors.Add(k);
                blocks[blockByStart[0]].Initial = [];
                foreach (int entry in handlerEntries)
                    blocks[blockByStart[entry]].Initial = [FlowValue.Described("catch")];
                foreach (Block block in blocks)
                    block.Entry = block.Initial;
                return blocks;
            }

            /// <summary>
            /// Marks the instructions that run whenever the method returns: those of the blocks that post-dominate the entry, so every path
            /// from the entry to a return passes through them. A path that ends in a throw never returns and constrains nothing, but code a
            /// handler guards may throw on any instruction, so its blocks also lead to the handler.
            /// </summary>
            private void MarkUnconditional(List<Block> blocks)
            {
                var all = Enumerable.Range(0, blocks.Count).ToHashSet();
                var successors = blocks.Select(block => block.Successors.Concat(HandlerBlocks(block, blocks)).Distinct().ToList()).ToArray();
                var postDominators = blocks.Select((block, k) => IsReturn(block) ? new HashSet<int> { k } : new HashSet<int>(all)).ToArray();
                for (bool changed = true; changed;)
                {
                    changed = false;
                    for (int k = blocks.Count - 1; k >= 0; k--)
                    {
                        if (successors[k].Count == 0)
                            continue;
                        var next = new HashSet<int>(postDominators[successors[k][0]]);
                        foreach (int successor in successors[k].Skip(1))
                            next.IntersectWith(postDominators[successor]);
                        next.Add(k);
                        if (!next.SetEquals(postDominators[k]))
                        {
                            postDominators[k] = next;
                            changed = true;
                        }
                    }
                }
                int entry = blocks.FindIndex(b => b.Start == 0);
                foreach (int k in postDominators[entry])
                    for (int index = blocks[k].Start; index < blocks[k].End; index++)
                        _straight.Add(index);
            }

            /// <summary>The blocks that start the handlers guarding any instruction of <paramref name="block"/>.</summary>
            private IEnumerable<int> HandlerBlocks(Block block, List<Block> blocks) =>
                body.Exceptions
                    .Where(handler => Enumerable.Range(block.Start, block.End - block.Start).Any(i => code[i].Offset >= handler.From && code[i].Offset < handler.To))
                    .Select(handler => blocks.FindIndex(b => b.Start == IndexOf(handler.Target)))
                    .Where(index => index >= 0);

            private bool IsReturn(Block block) => block.Successors.Count == 0 && code[block.End - 1].Opcode != AbcOpcodes.Throw;

            /// <summary>The blocks that dominate each block: every path from a root (the entry or a handler) to it passes through them.</summary>
            private static HashSet<int>[] Dominators(List<Block> blocks)
            {
                var all = Enumerable.Range(0, blocks.Count).ToHashSet();
                var dominators = blocks.Select((block, k) => block.Initial is not null ? new HashSet<int> { k } : new HashSet<int>(all)).ToArray();
                for (bool changed = true; changed;)
                {
                    changed = false;
                    for (int k = 0; k < blocks.Count; k++)
                    {
                        if (blocks[k].Initial is not null || blocks[k].Predecessors.Count == 0)
                            continue;
                        var next = new HashSet<int>(dominators[blocks[k].Predecessors[0]]);
                        foreach (int predecessor in blocks[k].Predecessors.Skip(1))
                            next.IntersectWith(dominators[predecessor]);
                        next.Add(k);
                        if (!next.SetEquals(dominators[k]))
                        {
                            dominators[k] = next;
                            changed = true;
                        }
                    }
                }
                return dominators;
            }

            /// <summary>The blocks control can come back to: a value merged there may differ on every turn of the loop.</summary>
            private static HashSet<int> BlocksInCycles(List<Block> blocks)
            {
                var inCycle = new HashSet<int>();
                for (int k = 0; k < blocks.Count; k++)
                {
                    var seen = new HashSet<int>();
                    var pending = new Stack<int>(blocks[k].Successors);
                    while (pending.Count > 0)
                    {
                        int next = pending.Pop();
                        if (next == k)
                        {
                            inCycle.Add(k);
                            break;
                        }
                        if (seen.Add(next))
                            blocks[next].Successors.ForEach(pending.Push);
                    }
                }
                return inCycle;
            }

            /// <summary>
            /// Propagates stacks to a fixed point. A block's entry is recomputed from the exits of all its predecessors each time one
            /// changes, so a merge only ever sees the current value of each path.
            /// </summary>
            private void Solve(List<Block> blocks)
            {
                var work = new Queue<int>(blocks.Select((b, k) => (b, k)).Where(p => p.b.Entry is not null).Select(p => p.k));
                while (work.Count > 0)
                {
                    int current = work.Dequeue();
                    Block block = blocks[current];
                    if (++block.Visits > MaxVisitsPerBlock)
                        throw new AbcFormatException($"{where}: the operand stack does not settle at offset {code[block.Start].Offset}.");
                    ImmutableList<FlowValue> exit = Transfer(block, block.Entry!);
                    if (block.Exit is not null && exit.SequenceEqual(block.Exit))
                        continue;
                    block.Exit = exit;
                    foreach (int successor in block.Successors.Distinct())
                    {
                        Block next = blocks[successor];
                        ImmutableList<FlowValue> entry = Join(blocks, successor);
                        if (next.Entry is null || !entry.SequenceEqual(next.Entry))
                        {
                            next.Entry = entry;
                            work.Enqueue(successor);
                        }
                    }
                }
            }

            /// <summary>The entry stack of a block: the stacks of every path into it that has been reached so far, merged slot by slot.</summary>
            private ImmutableList<FlowValue> Join(List<Block> blocks, int at)
            {
                Block block = blocks[at];
                var arriving = block.Predecessors.Distinct().Where(p => blocks[p].Exit is not null).Select(p => (Predecessor: (int?)p, Stack: blocks[p].Exit!)).ToList();
                if (block.Initial is not null)
                    arriving.Insert(0, (null, block.Initial));
                ImmutableList<FlowValue> first = arriving[0].Stack;
                if (arriving.Any(a => a.Stack.Count != first.Count))
                    throw new AbcFormatException($"{where}: the operand stack is {first.Count} deep on one path and {arriving.First(a => a.Stack.Count != first.Count).Stack.Count} on another at offset {code[block.Start].Offset}.");
                if (arriving.Count == 1)
                    return first;
                return Enumerable.Range(0, first.Count).Select(slot =>
                {
                    FlowValue merged = arriving[0].Stack[slot];
                    foreach ((_, ImmutableList<FlowValue> stack) in arriving.Skip(1))
                        merged = merged.Merge(stack[slot]);
                    return merged.Description == Unknown && ConditionalDescription(blocks, at, arriving, slot) is var described && described != Unknown
                        ? Redescribed(merged, described)
                        : merged;
                }).ToImmutableList();
            }

            private static FlowValue Redescribed(FlowValue value, string description) =>
                new(value.Origins, value.MaybeOther, description, value.Literal, value.Folded, value.Parts);

            /// <summary>
            /// The description of a value two paths give when they split at one conditional branch (a conditional expression, a short-circuit
            /// <c>&amp;&amp;</c> or <c>||</c>): <c>(condition?valueWhenTrue:valueWhenFalse)</c>. Any other merge of differently described values,
            /// and every merge inside a loop, cannot be described.
            /// </summary>
            private string ConditionalDescription(List<Block> blocks, int at, List<(int? Predecessor, ImmutableList<FlowValue> Stack)> arriving, int slot)
            {
                if (_inCycle.Contains(at) || arriving.Count != 2 || arriving.Any(a => a.Predecessor is null || a.Stack[slot].Description.Contains(Unknown, StringComparison.Ordinal)))
                    return Unknown;
                int? split = _dominators[at].Where(d => d != at).Select(d => (int?)d).MaxBy(d => _dominators[d!.Value].Count);
                if (split is not { } branch || !_conditions.TryGetValue(blocks[branch].End - 1, out var condition))
                    return Unknown;
                int fallthrough = blocks.FindIndex(b => b.Start == blocks[branch].End);
                int taken = blocks[branch].Successors.FirstOrDefault(s => s != fallthrough, -1);
                if (fallthrough < 0 || taken < 0)
                    return Unknown;

                bool? Taken(int predecessor) =>
                    predecessor == branch ? (at == taken) != (at == fallthrough) ? at == taken : null
                    : _dominators[predecessor].Contains(taken) != _dominators[predecessor].Contains(fallthrough) ? _dominators[predecessor].Contains(taken) : null;

                bool? firstTaken = Taken(arriving[0].Predecessor!.Value), secondTaken = Taken(arriving[1].Predecessor!.Value);
                if (firstTaken is null || secondTaken is null || firstTaken == secondTaken)
                    return Unknown;
                string takenValue = arriving[firstTaken.Value ? 0 : 1].Stack[slot].Description;
                string otherValue = arriving[firstTaken.Value ? 1 : 0].Stack[slot].Description;
                return condition.TakenWhenTrue
                    ? $"({condition.Condition}?{takenValue}:{otherValue})"
                    : $"({condition.Condition}?{otherValue}:{takenValue})";
            }

            private ImmutableList<FlowValue> Transfer(Block block, ImmutableList<FlowValue> entry)
            {
                var stack = new List<FlowValue>(entry);
                for (int i = block.Start; i < block.End; i++)
                    Step(code[i], i, stack);
                return [.. stack];
            }

            private List<FlowValue> Pop(List<FlowValue> stack, int count, AbcInstruction instruction)
            {
                if (count > stack.Count)
                    throw new AbcFormatException($"{where}: {instruction.Mnemonic} at offset {instruction.Offset} takes {count} values from a stack of {stack.Count}.");
                var popped = stack.GetRange(stack.Count - count, count);
                stack.RemoveRange(stack.Count - count, count);
                return popped;
            }

            private void Use(FlowValue value, string signature)
            {
                if (!_recording)
                    return;
                foreach (int origin in value.Origins)
                    _uses.Add((origin, signature));
                if (value.Parts is { } parts)
                    foreach ((string suffix, FlowValue part) in parts)
                        Use(part, signature + suffix);
            }

            private FlowValue Literal(int origin, object value, bool folded)
            {
                _literals[origin] = (value, folded);
                _originValues[origin] = value;
                return FlowValue.Constant(origin, value, folded);
            }

            private FlowValue Derived(int origin, object value, bool folded, params FlowValue[] inputs)
            {
                if (_recording)
                    foreach (FlowValue input in inputs)
                        _internal.UnionWith(input.Origins);
                return Literal(origin, value, folded);
            }

            private string Name(int multiname, List<FlowValue> runtime)
            {
                AbcMultiname name = abc.Multiname(multiname);
                return name.HasRuntimeName ? $"[{runtime[^1].Description}]" : abc.LocalName(multiname);
            }

            private static string Prefix(FlowValue receiver) =>
                receiver.Description is This or Scope ? "" : receiver.Description + ".";

            private string SlotName(int slot)
            {
                AbcTrait? trait = body.Traits.FirstOrDefault(t => t.SlotId == slot);
                return trait is null ? $"slot{slot}" : abc.LocalName(trait.Name);
            }

            /// <summary>Applies one instruction to the abstract stack, recording which signature each constant it takes flows into.</summary>
            private void Step(AbcInstruction instruction, int index, List<FlowValue> stack)
            {
                string mnemonic = instruction.Mnemonic;
                IReadOnlyList<int> operands = instruction.Operands;
                int offset = instruction.Offset;
                switch (mnemonic)
                {
                    case "pushbyte":
                        stack.Add(Literal(offset, operands[0], false));
                        return;
                    case "pushshort":
                        stack.Add(Literal(offset, (int)unchecked((short)operands[0]), false));
                        return;
                    case "pushint":
                        stack.Add(Literal(offset, PoolValue(abc.Ints, operands[0], "int"), false));
                        return;
                    case "pushuint":
                        stack.Add(Literal(offset, PoolValue(abc.UInts, operands[0], "uint"), false));
                        return;
                    case "pushdouble":
                        double number = PoolValue(abc.Doubles, operands[0], "double");
                        stack.Add(double.IsFinite(number) ? Literal(offset, number, false) : FlowValue.Described(AbcValues.Describe(number)));
                        return;
                    case "pushstring":
                        stack.Add(Literal(offset, abc.String(operands[0]), false));
                        return;
                    case "pushtrue":
                        stack.Add(Literal(offset, true, false));
                        return;
                    case "pushfalse":
                        stack.Add(Literal(offset, false, false));
                        return;
                    case "pushnull":
                        stack.Add(FlowValue.Described("null"));
                        return;
                    case "pushundefined":
                        stack.Add(FlowValue.Described("undefined"));
                        return;
                    case "pushnan":
                        stack.Add(FlowValue.Described("NaN"));
                        return;
                    case "pushnamespace":
                        stack.Add(FlowValue.Described("namespace"));
                        return;
                    case "getlocal0":
                        stack.Add(FlowValue.Described(This));
                        return;
                    case "getlocal1" or "getlocal2" or "getlocal3":
                        stack.Add(FlowValue.Described($"${mnemonic[^1]}"));
                        return;
                    case "getlocal":
                        stack.Add(FlowValue.Described($"${operands[0]}"));
                        return;
                    case "setlocal0" or "setlocal1" or "setlocal2" or "setlocal3":
                        Use(Pop(stack, 1, instruction)[0], $"${mnemonic[^1]}=");
                        return;
                    case "setlocal":
                        Use(Pop(stack, 1, instruction)[0], $"${operands[0]}=");
                        return;
                    case "dup":
                        {
                            FlowValue top = Pop(stack, 1, instruction)[0];
                            stack.Add(top);
                            stack.Add(top);
                            return;
                        }
                    case "swap":
                        {
                            var pair = Pop(stack, 2, instruction);
                            stack.Add(pair[1]);
                            stack.Add(pair[0]);
                            return;
                        }
                    case "pop":
                        Use(Pop(stack, 1, instruction)[0], "<pop>");
                        return;
                    case "findpropstrict" or "findproperty":
                        {
                            Pop(stack, abc.Multiname(operands[0]).RuntimeOperands, instruction).ForEach(v => Use(v, $"<{mnemonic}>"));
                            stack.Add(FlowValue.Described(Scope));
                            return;
                        }
                    case "finddef" or "getglobalscope" or "getscopeobject" or "getouterscope" or "newactivation":
                        stack.Add(FlowValue.Described(Scope));
                        return;
                    case "getlex":
                        stack.Add(FlowValue.Referenced(abc.LocalName(operands[0]), QualifiedReference(operands[0])));
                        return;
                    case "getproperty":
                        {
                            AbcMultiname name = abc.Multiname(operands[0]);
                            var values = Pop(stack, 1 + name.RuntimeOperands, instruction);
                            var runtime = values.Skip(1).ToList();
                            runtime.ForEach(v => Use(v, $"{values[0].Description}[]"));
                            string member = Name(operands[0], runtime);
                            stack.Add(FlowValue.Described(name.HasRuntimeName ? values[0].Description + member : Prefix(values[0]) + member));
                            return;
                        }
                    case "getsuper":
                        {
                            AbcMultiname name = abc.Multiname(operands[0]);
                            var values = Pop(stack, 1 + name.RuntimeOperands, instruction);
                            values.Skip(1).ToList().ForEach(v => Use(v, "super[]"));
                            stack.Add(FlowValue.Described("super." + Name(operands[0], values.Skip(1).ToList())));
                            return;
                        }
                    case "setproperty" or "initproperty" or "setsuper":
                        {
                            AbcMultiname name = abc.Multiname(operands[0]);
                            var values = Pop(stack, 2 + name.RuntimeOperands, instruction);
                            FlowValue receiver = values[0];
                            var runtime = values.Skip(1).Take(name.RuntimeOperands).ToList();
                            FlowValue value = values[^1];
                            string member = Name(operands[0], runtime);
                            string target = mnemonic == "setsuper" ? "super." + member
                                : name.HasRuntimeName ? receiver.Description + member : Prefix(receiver) + member;
                            runtime.ForEach(v => Use(v, $"{receiver.Description}[]="));
                            Use(value, target + "=");
                            if (mnemonic != "setsuper" && !name.HasRuntimeName && receiver.Description is This or Scope)
                                RecordAssignment(member, value, index);
                            return;
                        }
                    case "deleteproperty":
                        {
                            AbcMultiname name = abc.Multiname(operands[0]);
                            Pop(stack, 1 + name.RuntimeOperands, instruction).ForEach(v => Use(v, "<delete>"));
                            stack.Add(FlowValue.Other);
                            return;
                        }
                    case "getslot":
                        {
                            FlowValue receiver = Pop(stack, 1, instruction)[0];
                            stack.Add(FlowValue.Described(Prefix(receiver) + SlotName(operands[0])));
                            return;
                        }
                    case "setslot":
                        {
                            var values = Pop(stack, 2, instruction);
                            Use(values[1], Prefix(values[0]) + SlotName(operands[0]) + "=");
                            return;
                        }
                    case "getglobalslot":
                        stack.Add(FlowValue.Described($"global.slot{operands[0]}"));
                        return;
                    case "setglobalslot":
                        Use(Pop(stack, 1, instruction)[0], $"global.slot{operands[0]}=");
                        return;
                    case "callproperty" or "callproplex" or "callpropvoid" or "callsuper" or "callsupervoid" or "constructprop":
                        {
                            AbcMultiname name = abc.Multiname(operands[0]);
                            int argumentCount = operands[1];
                            var values = Pop(stack, 1 + name.RuntimeOperands + argumentCount, instruction);
                            FlowValue receiver = values[0];
                            var runtime = values.Skip(1).Take(name.RuntimeOperands).ToList();
                            string member = Name(operands[0], runtime);
                            string callee = mnemonic.StartsWith("callsuper", StringComparison.Ordinal) ? "super." + member
                                : name.HasRuntimeName ? receiver.Description + member : Prefix(receiver) + member;
                            if (mnemonic == "constructprop")
                                callee = "new " + callee;
                            runtime.ForEach(v => Use(v, $"{receiver.Description}[]()"));
                            var arguments = values.Skip(1 + name.RuntimeOperands).ToList();
                            for (int a = 0; a < arguments.Count; a++)
                                Use(arguments[a], $"{callee}({a})");
                            if (!mnemonic.StartsWith("callsuper", StringComparison.Ordinal) && mnemonic != "constructprop" && !name.HasRuntimeName
                                && receiver.Description is This or Scope)
                                RecordOwnCall(member, arguments, index);
                            if (!mnemonic.EndsWith("void", StringComparison.Ordinal))
                                stack.Add(Called(callee));
                            return;
                        }
                    case "call":
                        {
                            var values = Pop(stack, 2 + operands[0], instruction);
                            string callee = values[0].Description;
                            Use(values[1], $"<call receiver>");
                            for (int a = 2; a < values.Count; a++)
                                Use(values[a], $"{callee}({a - 2})");
                            stack.Add(Called(callee));
                            return;
                        }
                    case "construct" or "applytype":
                        {
                            var values = Pop(stack, 1 + operands[0], instruction);
                            string callee = (mnemonic == "construct" ? "new " : "") + values[0].Description;
                            for (int a = 1; a < values.Count; a++)
                                Use(values[a], $"{callee}({a - 1})");
                            stack.Add(Called(callee));
                            return;
                        }
                    case "constructsuper":
                        {
                            var values = Pop(stack, 1 + operands[0], instruction);
                            for (int a = 1; a < values.Count; a++)
                                Use(values[a], $"super({a - 1})");
                            return;
                        }
                    case "callmethod" or "callstatic":
                        {
                            var values = Pop(stack, 1 + operands[1], instruction);
                            string callee = $"{mnemonic}{operands[0]}";
                            for (int a = 1; a < values.Count; a++)
                                Use(values[a], $"{callee}({a - 1})");
                            stack.Add(Called(callee));
                            return;
                        }
                    case "newarray":
                        {
                            var values = Pop(stack, operands[0], instruction);
                            stack.Add(FlowValue.Composite("[]", [.. values.Select((v, k) => ($"[{k}]", v))]));
                            return;
                        }
                    case "newobject":
                        {
                            var values = Pop(stack, operands[0] * 2, instruction);
                            var parts = ImmutableArray.CreateBuilder<(string, FlowValue)>();
                            for (int k = 0; k < values.Count; k += 2)
                            {
                                FlowValue key = values[k];
                                if (key.Literal is string)
                                {
                                    if (_recording)
                                        _internal.UnionWith(key.Origins);
                                }
                                else
                                    Use(key, "<object key>");
                                parts.Add((key.Literal is string property ? "." + property : $"[{Unknown}]", values[k + 1]));
                            }
                            stack.Add(FlowValue.Composite("{}", parts.ToImmutable()));
                            return;
                        }
                    case "newfunction":
                        stack.Add(FlowValue.Described("function"));
                        return;
                    case "newclass":
                        Use(Pop(stack, 1, instruction)[0], "<newclass>");
                        stack.Add(FlowValue.Described("class"));
                        return;
                    case "newcatch":
                        stack.Add(FlowValue.Described("catch"));
                        return;
                    case "returnvalue":
                        Use(Pop(stack, 1, instruction)[0], "return");
                        return;
                    case "throw":
                        Use(Pop(stack, 1, instruction)[0], "throw");
                        return;
                    case "iftrue" or "iffalse":
                        {
                            FlowValue tested = Pop(stack, 1, instruction)[0];
                            Use(tested, "if");
                            _conditions[index] = (tested.Description, mnemonic == "iftrue");
                            return;
                        }
                    case "lookupswitch":
                        Use(Pop(stack, 1, instruction)[0], "switch");
                        return;
                    case "pushscope" or "pushwith":
                        Use(Pop(stack, 1, instruction)[0], $"<{mnemonic}>");
                        return;
                }

                if (ComparisonBranches.TryGetValue(mnemonic, out string? comparison))
                {
                    var pair = Pop(stack, 2, instruction);
                    UseOperands(pair, comparison);
                    _conditions[index] = ($"({pair[0].Description}{comparison}{pair[1].Description})", !BranchesWhenFalse.Contains(mnemonic));
                    return;
                }
                if (BinaryOperators.TryGetValue(mnemonic, out string? symbol))
                {
                    var pair = Pop(stack, 2, instruction);
                    if (pair[0].Literal is { } left && pair[1].Literal is { } right && AbcValues.FoldBinary(mnemonic, left, right) is { } folded)
                    {
                        stack.Add(Derived(offset, folded, true, pair[0], pair[1]));
                        return;
                    }
                    UseOperands(pair, symbol);
                    stack.Add(FlowValue.Described($"({pair[0].Description}{symbol}{pair[1].Description})"));
                    return;
                }
                if (KeywordOperators.TryGetValue(mnemonic, out string? keyword))
                {
                    var pair = Pop(stack, 2, instruction);
                    UseOperands(pair, keyword);
                    stack.Add(FlowValue.Described($"({pair[0].Description}{keyword}{pair[1].Description})"));
                    return;
                }
                if (UnaryOperators.TryGetValue(mnemonic, out string? format))
                {
                    FlowValue operand = Pop(stack, 1, instruction)[0];
                    if (operand.Literal is { } value && AbcValues.FoldUnary(mnemonic, value) is { } folded)
                    {
                        stack.Add(Derived(offset, folded, true, operand));
                        return;
                    }
                    Use(operand, $"<{mnemonic}>");
                    stack.Add(FlowValue.Described(string.Format(System.Globalization.CultureInfo.InvariantCulture, format, operand.Description)));
                    return;
                }
                if (mnemonic == "istype")
                {
                    FlowValue operand = Pop(stack, 1, instruction)[0];
                    Use(operand, "<istype>");
                    stack.Add(FlowValue.Described($"({operand.Description} is {abc.LocalName(operands[0])})"));
                    return;
                }
                if (mnemonic.StartsWith("convert_", StringComparison.Ordinal) || mnemonic.StartsWith("coerce", StringComparison.Ordinal) || mnemonic == "astype")
                {
                    stack.Add(Convert(instruction, Pop(stack, 1, instruction)[0]));
                    return;
                }
                ApplyGenericEffect(instruction, stack);
            }

            /// <summary>
            /// A conversion keeps the value when it changes none of the constants that may reach it; a constant it changes becomes the
            /// converted constant, and a value whose constants it may change, or whose conversion is not certain, becomes a computed value.
            /// </summary>
            private FlowValue Convert(AbcInstruction instruction, FlowValue operand)
            {
                string mnemonic = instruction.Mnemonic;
                string? type = mnemonic is "coerce" or "astype" ? abc.LocalName(instruction.Operands[0]) : null;
                if (operand.Literal is { } value && AbcValues.Convert(mnemonic, type, value) is { } converted)
                    return Unchanged(converted, value) ? operand : Derived(instruction.Offset, converted, true, operand);
                if (operand.Literal is null && operand.Origins.All(o => AbcValues.Convert(mnemonic, type, _originValues[o]) is { } c && Unchanged(c, _originValues[o])))
                    return operand;
                Use(operand, $"<{mnemonic}>");
                return FlowValue.Described($"{type ?? ConversionNames.GetValueOrDefault(mnemonic, mnemonic)}({operand.Description})");
            }

            private static bool Unchanged(object converted, object value) => converted.GetType() == value.GetType() && Equals(converted, value);

            /// <summary>The result of a call, described by its callee alone so the key does not carry the argument values.</summary>
            private static FlowValue Called(string callee) => FlowValue.Described(callee + "()");

            private void UseOperands(List<FlowValue> pair, string symbol)
            {
                Use(pair[0], symbol + pair[1].Description);
                Use(pair[1], pair[0].Description + symbol);
            }

            private void RecordAssignment(string member, FlowValue value, int index)
            {
                if (!_recording)
                    return;
                AbcLiteral? literal = value.Literal is { } constant ? new AbcLiteral(value.Origins.Single(), constant, value.Folded) : null;
                _assignments.Add(new AbcMemberAssignment(member, literal, _straight.Contains(index), References(value)));
            }

            private static List<string>? References(FlowValue value) =>
                value.Parts is { Length: > 0 } parts && value.Description == "[]" && parts.All(part => part.Value.Reference is not null)
                    ? parts.Select(part => part.Value.Reference!).ToList()
                    : null;

            private string QualifiedReference(int multiname) =>
                abc.Multiname(multiname).Kind is AbcMultinameKind.QName or AbcMultinameKind.QNameA or AbcMultinameKind.Multiname or AbcMultinameKind.MultinameA
                    ? abc.QualifiedName(multiname)
                    : abc.LocalName(multiname);

            private void RecordOwnCall(string method, List<FlowValue> arguments, int index)
            {
                if (!_recording)
                    return;
                _ownCalls.Add(new AbcOwnCall(method, arguments.Select(a => new AbcCallArgument(a.Description, a.Literal)).ToList(), _straight.Contains(index)));
            }

            /// <summary>Instructions that neither push constants nor name their operands: their pops and pushes by the AVM2 stack rules.</summary>
            private void ApplyGenericEffect(AbcInstruction instruction, List<FlowValue> stack)
            {
                (int pops, int pushes) = instruction.Mnemonic switch
                {
                    "nop" or "bkpt" or "label" or "kill" or "dxns" or "debug" or "debugline" or "debugfile" or "bkptline" or "timestamp"
                        or "jump" or "returnvoid" or "popscope" or "inclocal" or "declocal" or "inclocal_i" or "declocal_i" => (0, 0),
                    "dxnslate" => (1, 0),
                    "hasnext2" => (0, 1),
                    "nextname" or "nextvalue" or "hasnext" => (2, 1),
                    "getdescendants" => (1 + abc.Multiname(instruction.Operands[0]).RuntimeOperands, 1),
                    "li8" or "li16" or "li32" or "lf32" or "lf64" or "sxi1" or "sxi8" or "sxi16" or "esc_xelem" or "esc_xattr" or "checkfilter" => (1, 1),
                    "si8" or "si16" or "si32" or "sf32" or "sf64" => (2, 0),
                    _ => throw new AbcFormatException($"{where}: {instruction.Mnemonic} at offset {instruction.Offset} has no stack rule."),
                };
                Pop(stack, pops, instruction).ForEach(v => Use(v, $"<{instruction.Mnemonic}>"));
                for (int i = 0; i < pushes; i++)
                    stack.Add(FlowValue.Other);
            }

            private T PoolValue<T>(IReadOnlyList<T> pool, int index, string what) =>
                index > 0 && index < pool.Count ? pool[index] : throw new AbcFormatException($"{where}: {what} pool index {index} is outside the pool of {pool.Count}.");
        }

        private static readonly IReadOnlyDictionary<string, string> BinaryOperators = new Dictionary<string, string>
        {
            ["add"] = "+",
            ["add_i"] = "+",
            ["subtract"] = "-",
            ["subtract_i"] = "-",
            ["multiply"] = "*",
            ["multiply_i"] = "*",
            ["divide"] = "/",
            ["modulo"] = "%",
            ["lshift"] = "<<",
            ["rshift"] = ">>",
            ["urshift"] = ">>>",
            ["bitand"] = "&",
            ["bitor"] = "|",
            ["bitxor"] = "^",
            ["equals"] = "==",
            ["strictequals"] = "===",
            ["lessthan"] = "<",
            ["lessequals"] = "<=",
            ["greaterthan"] = ">",
            ["greaterequals"] = ">=",
        };

        private static readonly IReadOnlyDictionary<string, string> KeywordOperators = new Dictionary<string, string>
        {
            ["instanceof"] = " instanceof ",
            ["in"] = " in ",
            ["istypelate"] = " is ",
            ["astypelate"] = " as ",
        };

        /// <summary>One-operand operators, as a format of their operand's description.</summary>
        private static readonly IReadOnlyDictionary<string, string> UnaryOperators = new Dictionary<string, string>
        {
            ["negate"] = "-({0})",
            ["negate_i"] = "-({0})",
            ["bitnot"] = "~({0})",
            ["not"] = "!({0})",
            ["increment"] = "({0}+1)",
            ["increment_i"] = "({0}+1)",
            ["decrement"] = "({0}-1)",
            ["decrement_i"] = "({0}-1)",
            ["typeof"] = "typeof({0})",
        };

        private static readonly IReadOnlyDictionary<string, string> ConversionNames = new Dictionary<string, string>
        {
            ["convert_i"] = "int",
            ["coerce_i"] = "int",
            ["convert_u"] = "uint",
            ["coerce_u"] = "uint",
            ["convert_d"] = "Number",
            ["coerce_d"] = "Number",
            ["convert_s"] = "String",
            ["coerce_s"] = "String",
            ["convert_b"] = "Boolean",
            ["coerce_b"] = "Boolean",
        };

        /// <summary>The compare-and-branch instructions that branch when their comparison is false.</summary>
        private static readonly HashSet<string> BranchesWhenFalse = new(StringComparer.Ordinal) { "ifne", "ifstrictne", "ifnlt", "ifnle", "ifngt", "ifnge" };

        /// <summary>Fused compare-and-branch instructions, named by the comparison they test whichever way the branch goes.</summary>
        private static readonly IReadOnlyDictionary<string, string> ComparisonBranches = new Dictionary<string, string>
        {
            ["ifeq"] = "==",
            ["ifne"] = "==",
            ["ifstricteq"] = "===",
            ["ifstrictne"] = "===",
            ["iflt"] = "<",
            ["ifnlt"] = "<",
            ["ifle"] = "<=",
            ["ifnle"] = "<=",
            ["ifgt"] = ">",
            ["ifngt"] = ">",
            ["ifge"] = ">=",
            ["ifnge"] = ">=",
        };
    }
}
