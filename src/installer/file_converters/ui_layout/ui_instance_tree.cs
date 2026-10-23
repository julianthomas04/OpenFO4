using AssetConverter.Swf;

namespace AssetConverter.UiLayout
{
    /// <summary>A change of an object's placement within its life, at a frame of its parent's timeline.</summary>
    internal sealed record UiTrackKey(int Frame, SwfDisplayObject State);

    /// <summary>
    /// One addressable display object: its path, what it shows, where its life on the parent timeline begins and ends, its placement
    /// at the frame the path views, that placement composed up to the path's root, and the placement changes within its life.
    /// </summary>
    internal sealed record UiInstance(
        string Path,
        int CharacterId,
        string Kind,
        int FirstFrame,
        int LastFrame,
        SwfDisplayObject Local,
        SwfMatrix WorldMatrix,
        ColorTransform WorldColor,
        IReadOnlyList<UiTrackKey> Track);

    internal sealed record UiInstanceTree(IReadOnlyList<UiInstance> Instances, IReadOnlyDictionary<string, string> Unresolved);

    /// <summary>
    /// Walks a movie's display lists into instance paths. A path is the chain of instance names from a root: the main timeline (no prefix)
    /// or an exported symbol (<c>[Class]</c>). An unnamed object is <c>@depth</c>. A sprite's children are listed at its frame 1 under the
    /// plain segment, at each labelled frame under <c>segment:Label</c>, and at every other frame where objects first appear under
    /// <c>segment:N</c> (those objects only). Two objects of one display list with the same name leave
    /// that path unresolved; both stay reachable by depth.
    /// </summary>
    internal sealed class UiInstanceTreeBuilder
    {
        private readonly SwfMovie _movie;
        private readonly Dictionary<int, SwfPlayedTimeline> _played = [];
        private readonly Dictionary<SwfPlayedTimeline, Dictionary<int, (int First, int Last, List<UiTrackKey> Track)>> _lives = new(ReferenceEqualityComparer.Instance);
        private readonly List<UiInstance> _instances = [];
        private readonly HashSet<int> _walking = [];
        private readonly SortedDictionary<string, string> _unresolved = new(StringComparer.Ordinal);

        private UiInstanceTreeBuilder(SwfMovie movie)
        {
            _movie = movie;
        }

        public const string LabelSeparator = ":";
        public const string DepthPrefix = "@";

        public static UiInstanceTree Build(SwfMovie movie, IEnumerable<(string Class, int CharacterId)> exportedSymbols)
        {
            var builder = new UiInstanceTreeBuilder(movie);
            builder.WalkTimeline("", SwfTimelinePlayer.Play(movie.Root, "the main timeline"), SwfMatrix.Identity, ColorTransform.Identity);
            foreach ((string className, int characterId) in exportedSymbols)
                if (movie.Characters.GetValueOrDefault(characterId) is SwfSprite)
                    builder.WalkSprite(SymbolRoot(className), characterId, SwfMatrix.Identity, ColorTransform.Identity);
            return new UiInstanceTree(builder._instances, builder._unresolved);
        }

        public const string ImportKind = "import";

        private string KindOf(SwfCharacter? character, SwfDisplayObject placed) =>
            character?.Kind ?? (_movie.Imports.ContainsKey(placed.CharacterId)
                ? ImportKind
                : throw new SwfFormatException($"The object at depth {placed.Depth} places character {placed.CharacterId}, which the movie neither defines nor imports."));

        private void WalkSprite(string prefix, int spriteId, SwfMatrix world, ColorTransform worldColor)
        {
            if (!_walking.Add(spriteId))
                throw new SwfFormatException($"Sprite {spriteId} contains itself.");
            WalkTimeline(prefix, Played(spriteId), world, worldColor);
            _walking.Remove(spriteId);
        }

        public static string SymbolRoot(string className) => $"[{className}]";

        public SwfPlayedTimeline Played(int spriteId)
        {
            if (_played.TryGetValue(spriteId, out SwfPlayedTimeline? played))
                return played;
            if (_movie.Characters.GetValueOrDefault(spriteId) is not SwfSprite sprite)
                throw new SwfFormatException($"Character {spriteId} is not a sprite.");
            played = SwfTimelinePlayer.Play(sprite.Timeline, $"sprite {spriteId}");
            _played[spriteId] = played;
            return played;
        }

        private void WalkTimeline(string prefix, SwfPlayedTimeline timeline, SwfMatrix world, ColorTransform worldColor)
        {
            if (timeline.FrameCount == 0)
                return;
            WalkFrame(prefix, timeline, 1, world, worldColor, onlyNew: false);
            var labelled = new HashSet<int>(timeline.Labels.Values);
            foreach ((string label, int frame) in timeline.Labels.OrderBy(l => l.Value).ThenBy(l => l.Key, StringComparer.Ordinal))
                WalkFrame(prefix + LabelSeparator + label, timeline, frame, world, worldColor, onlyNew: false);
            var lives = Lives(timeline);
            foreach (int frame in lives.Values.Select(l => l.First).Where(f => f > 1 && !labelled.Contains(f)).Distinct().Order())
                WalkFrame(prefix + LabelSeparator + UiLayoutJson.Invariant(frame), timeline, frame, world, worldColor, onlyNew: true);
        }

        /// <param name="onlyNew">List only the objects whose life begins at this frame (an unlabelled keyframe view).</param>
        private void WalkFrame(string viewPrefix, SwfPlayedTimeline timeline, int frame, SwfMatrix world, ColorTransform worldColor, bool onlyNew)
        {
            var lives = Lives(timeline);
            IReadOnlyList<SwfDisplayObject> displayList = onlyNew
                ? timeline.Frames[frame - 1].Where(o => lives[o.Life].First == frame).ToList()
                : timeline.Frames[frame - 1];
            var duplicateNames = displayList.Where(o => o.Name is not null).GroupBy(o => o.Name!, StringComparer.Ordinal)
                .Where(g => g.Count() > 1).ToDictionary(g => g.Key, g => g.Count(), StringComparer.Ordinal);
            foreach ((string name, int count) in duplicateNames)
                _unresolved[Join(viewPrefix, name)] = $"{count} instances are named '{name}' at frame {frame}";

            foreach (SwfDisplayObject placed in displayList)
            {
                string segment = placed.Name is { } name && !duplicateNames.ContainsKey(name) ? name : DepthPrefix + placed.Depth;
                string path = Join(viewPrefix, segment);
                SwfMatrix childWorld = world.Compose(placed.Matrix);
                ColorTransform childColor = worldColor.Compose(placed.Color);
                (int first, int last, List<UiTrackKey> track) = lives[placed.Life];
                SwfCharacter? character = _movie.Characters.GetValueOrDefault(placed.CharacterId);
                _instances.Add(new UiInstance(path, placed.CharacterId, KindOf(character, placed), first, last, placed, childWorld, childColor, track));
                if (character is SwfSprite)
                    WalkSprite(path, placed.CharacterId, childWorld, childColor);
            }
        }

        /// <summary>For each object of a timeline: the frames it lives on and, when its placement changes during that life, each change (the first frame included).</summary>
        private Dictionary<int, (int First, int Last, List<UiTrackKey> Track)> Lives(SwfPlayedTimeline timeline)
        {
            if (_lives.TryGetValue(timeline, out var known))
                return known;
            var lives = new Dictionary<int, (int First, int Last, List<UiTrackKey> Track)>();
            for (int f = 0; f < timeline.FrameCount; f++)
            {
                foreach (SwfDisplayObject state in timeline.Frames[f])
                {
                    if (!lives.TryGetValue(state.Life, out var life))
                        life = (f + 1, f + 1, []);
                    if (life.Track.Count == 0 || !life.Track[^1].State.PlacementEquals(state))
                        life.Track.Add(new UiTrackKey(f + 1, state));
                    lives[state.Life] = (life.First, f + 1, life.Track);
                }
            }
            foreach (int life in lives.Keys.ToList())
                if (lives[life].Track.Count == 1)
                    lives[life] = (lives[life].First, lives[life].Last, []);
            _lives[timeline] = lives;
            return lives;
        }

        private static string Join(string prefix, string segment) => prefix.Length == 0 ? segment : $"{prefix}/{segment}";
    }
}
