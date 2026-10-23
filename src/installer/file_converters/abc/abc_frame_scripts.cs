namespace AssetConverter.Abc
{
    internal sealed class FrameScriptException(string message) : Exception(message);

    internal enum FrameScriptActionKind
    {
        Stop,
        Play,
        GotoAndStop,
        GotoAndPlay,
        NextFrame,
        PrevFrame,
    }

    /// <summary>A timeline call a frame script makes on its own timeline. <see cref="Target"/> is the frame number or label a goto names.</summary>
    internal sealed record FrameScriptAction(FrameScriptActionKind Kind, object? Target);

    /// <summary>
    /// Reads what the frame scripts of a timeline class do to their own timeline: the class constructor registers each frame's method with
    /// <c>addFrameScript(frameIndex, method, ...)</c>, and each method may call <c>stop</c>, <c>play</c>, <c>gotoAndStop</c>,
    /// <c>gotoAndPlay</c>, <c>nextFrame</c> or <c>prevFrame</c>. A registration or timeline call that is conditional, or whose frame
    /// or method is not a constant, throws <see cref="FrameScriptException"/>: the timeline's behaviour is then not fixed by the data.
    /// Malformed bytecode throws <see cref="AbcFormatException"/>.
    /// </summary>
    internal static class AbcFrameScripts
    {
        private const string Register = "addFrameScript";

        private static readonly Dictionary<string, FrameScriptActionKind> TimelineCalls = new Dictionary<string, FrameScriptActionKind>(StringComparer.Ordinal)
        {
            ["stop"] = FrameScriptActionKind.Stop,
            ["play"] = FrameScriptActionKind.Play,
            ["gotoAndStop"] = FrameScriptActionKind.GotoAndStop,
            ["gotoAndPlay"] = FrameScriptActionKind.GotoAndPlay,
            ["nextFrame"] = FrameScriptActionKind.NextFrame,
            ["prevFrame"] = FrameScriptActionKind.PrevFrame,
        };

        /// <returns>The timeline calls by frame (1-based), in the order each frame's script makes them; frames without a script are absent.</returns>
        public static IReadOnlyDictionary<int, IReadOnlyList<FrameScriptAction>> Of(IReadOnlyList<AbcFile> blocks, string className, string where)
        {
            var definitions = blocks.SelectMany(abc => abc.Instances.Select((instance, index) => (abc, instance, index)))
                .Where(d => d.abc.QualifiedName(d.instance.Name) == className).ToList();
            if (definitions.Count != 1)
                throw new FrameScriptException($"{where}: the class '{className}' is defined {definitions.Count} times; exactly once is required.");
            (AbcFile abc, AbcInstance instance, _) = definitions[0];
            var bodies = abc.Bodies.ToDictionary(b => b.Method);
            var methods = instance.Traits.Where(t => t.Kind == AbcTraitKind.Method).ToDictionary(t => abc.LocalName(t.Name), t => t.Target, StringComparer.Ordinal);

            var scripts = new SortedDictionary<int, IReadOnlyList<FrameScriptAction>>();
            if (!bodies.TryGetValue(instance.Initializer, out AbcMethodBody? constructor))
                return scripts;
            foreach (AbcOwnCall registration in AbcLiteralFlow.Analyze(abc, constructor, $"{where}, {className} constructor").OwnCalls.Where(c => c.Method == Register))
            {
                if (!registration.Unconditional)
                    throw new FrameScriptException($"{where}: {className} registers a frame script conditionally.");
                if (registration.Arguments.Count % 2 != 0)
                    throw new FrameScriptException($"{where}: {className} calls {Register} with {registration.Arguments.Count} arguments, which pair no frame with a method.");
                for (int a = 0; a < registration.Arguments.Count; a += 2)
                {
                    int frame = FrameIndex(registration.Arguments[a], className, where) + 1;
                    string method = registration.Arguments[a + 1].Description;
                    if (!methods.TryGetValue(method, out int methodIndex) || !bodies.TryGetValue(methodIndex, out AbcMethodBody? body))
                        throw new FrameScriptException($"{where}: {className} registers '{method}' for frame {frame}, which is not one of its methods.");
                    if (!scripts.TryAdd(frame, Actions(abc, body, $"{className}.{method}", where)))
                        throw new FrameScriptException($"{where}: {className} registers frame {frame} twice.");
                }
            }
            return scripts;
        }

        private static int FrameIndex(AbcCallArgument argument, string className, string where) =>
            argument.Literal is int index && index >= 0
                ? index
                : throw new FrameScriptException($"{where}: {className} registers a frame script for '{argument.Description}', which is not a frame index.");

        private static List<FrameScriptAction> Actions(AbcFile abc, AbcMethodBody body, string method, string where)
        {
            var actions = new List<FrameScriptAction>();
            foreach (AbcOwnCall call in AbcLiteralFlow.Analyze(abc, body, $"{where}, {method}").OwnCalls)
            {
                if (!TimelineCalls.TryGetValue(call.Method, out FrameScriptActionKind kind))
                    continue;
                if (!call.Unconditional)
                    throw new FrameScriptException($"{where}: {method} calls {call.Method} conditionally.");
                bool takesFrame = kind is FrameScriptActionKind.GotoAndStop or FrameScriptActionKind.GotoAndPlay;
                if (call.Arguments.Count != (takesFrame ? 1 : 0))
                    throw new FrameScriptException($"{where}: {method} calls {call.Method} with {call.Arguments.Count} arguments.");
                object? target = takesFrame ? call.Arguments[0].Literal : null;
                if (takesFrame && target is not (int or string))
                    throw new FrameScriptException($"{where}: {method} calls {call.Method}({call.Arguments[0].Description}), which names no constant frame.");
                actions.Add(new FrameScriptAction(kind, target));
            }
            return actions;
        }
    }
}
