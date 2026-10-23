using AssetConverter.Abc;
using AssetConverter.Swf;

namespace AssetConverter
{
    /// <summary>
    /// A timeline call a clip's frame script makes on its own timeline: on clip frame <paramref name="Frame"/> (1-based) it calls
    /// <paramref name="Action"/> (<c>stop</c>, <c>play</c>, <c>gotoAndStop</c>, <c>gotoAndPlay</c>, <c>nextFrame</c> or
    /// <c>prevFrame</c>), a goto with the clip frame <paramref name="Target"/> it goes to.
    /// </summary>
    public sealed record ClipScriptStep(int Frame, string Action, int? Target);

    /// <summary>
    /// The frame scripts of a movie's timelines as clip steps. A timeline has frame scripts only through its class (the document class
    /// for the main timeline, the symbol class for a sprite), so a timeline without one has none and simply plays on, wrapping round.
    /// Labels a goto names are resolved to frame numbers. Throws <see cref="FrameScriptException"/> when the scripts are not fixed by the
    /// data or reach outside the clip.
    /// </summary>
    internal sealed class PipboyArtScripts(SwfMovie movie, string source)
    {
        const int DocumentCharacter = 0;

        static readonly Dictionary<FrameScriptActionKind, string> ActionNames = new()
        {
            [FrameScriptActionKind.Stop] = "stop",
            [FrameScriptActionKind.Play] = "play",
            [FrameScriptActionKind.GotoAndStop] = "gotoAndStop",
            [FrameScriptActionKind.GotoAndPlay] = "gotoAndPlay",
            [FrameScriptActionKind.NextFrame] = "nextFrame",
            [FrameScriptActionKind.PrevFrame] = "prevFrame",
        };

        readonly Lazy<IReadOnlyList<AbcFile>> _blocks = new(() => movie.AbcBlocks.Select(block => AbcReader.Read(block.Data)).ToList());

        /// <param name="spriteId">The sprite whose timeline the clip shows, or <c>null</c> for the main timeline.</param>
        /// <param name="frames">The consecutive timeline frames (1-based) the clip is made of.</param>
        public IReadOnlyList<ClipScriptStep> Of(int? spriteId, IReadOnlyList<int> frames)
        {
            if (frames.Count == 0 || frames.Select((frame, index) => frame - index).Distinct().Count() != 1)
                throw new ArgumentException("A clip's frames must be consecutive.", nameof(frames));
            var classes = movie.SymbolClasses.Where(symbol => symbol.CharacterId == (spriteId ?? DocumentCharacter)).Select(symbol => symbol.Name).Distinct().ToList();
            if (classes.Count == 0)
                return [];
            string where = spriteId is { } id ? $"'{source}' sprite {id}" : $"the main timeline of '{source}'";
            if (classes.Count > 1)
                throw new FrameScriptException($"{where} has {classes.Count} classes; exactly one is required.");
            SwfTimelineTags timeline = spriteId is { } sprite
                ? (movie.Characters.GetValueOrDefault(sprite) as SwfSprite)?.Timeline ?? throw new FrameScriptException($"{where} is not a sprite.")
                : movie.Root;

            int first = frames[0], last = frames[^1];
            var steps = new List<ClipScriptStep>();
            foreach ((int frame, IReadOnlyList<FrameScriptAction> actions) in AbcFrameScripts.Of(_blocks.Value, classes[0], source))
            {
                if (frame > timeline.Frames.Count)
                    throw new FrameScriptException($"{where} has a frame script for frame {frame} of its {timeline.Frames.Count}.");
                if (frame < first || frame > last)
                    continue;
                foreach (FrameScriptAction action in actions)
                {
                    int? target = action.Target switch
                    {
                        null => null,
                        int number => number,
                        string label => FrameOfLabel(timeline, label, where),
                        _ => throw new FrameScriptException($"{where}: frame {frame} goes to '{action.Target}', which is neither a frame number nor a label."),
                    };
                    if (target is { } goesTo && (goesTo < first || goesTo > last))
                        throw new FrameScriptException($"{where}: frame {frame} goes to frame {target}, outside the clip's frames {first}..{last}.");
                    steps.Add(new ClipScriptStep(frame - first + 1, ActionNames[action.Kind], target - first + 1));
                }
            }
            return steps;
        }

        static int FrameOfLabel(SwfTimelineTags timeline, string label, string where)
        {
            var frames = timeline.Frames.Select((frame, index) => (Number: index + 1, frame.Labels)).Where(f => f.Labels.Contains(label)).ToList();
            return frames.Count == 1
                ? frames[0].Number
                : throw new FrameScriptException($"{where}: {frames.Count} frames are labelled '{label}'; a goto needs exactly one.");
        }
    }
}
