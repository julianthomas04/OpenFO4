using AssetConverter.Swf;

namespace AssetConverter
{
    /// <summary>
    /// The structure of a movie for resolving symbols by class, class prefix, instance name or frame label instead of by character id,
    /// and for the tight bounds of everything a timeline ever shows. Timelines are read as <see cref="SwfTimelinePlayer"/> plays them, so a
    /// placement that only modifies an object keeps what it does not restate. Every lookup that finds nothing or finds more than one throws
    /// <see cref="SwfFormatException"/>.
    /// </summary>
    internal sealed class PipboySwfTree(SwfMovie movie)
    {
        readonly Dictionary<int, SwfPlayedTimeline> _played = [];
        readonly Dictionary<int, TwipRect?> _extentCache = [];
        readonly Dictionary<string, int> _characterByClass = new(StringComparer.Ordinal);

        public SwfMovie Movie => movie;

        public int CharacterOfClass(string className)
        {
            if (_characterByClass.Count == 0)
                foreach ((int id, string name) in movie.SymbolClasses)
                    _characterByClass[name] = id;
            return _characterByClass.TryGetValue(className, out int character) ? character : throw new SwfFormatException($"Symbol class '{className}' is not in the SWF.");
        }

        /// <summary>The character of the only symbol whose class starts with <paramref name="prefix"/>, for auto-numbered classes.</summary>
        public int CharacterOfClassPrefix(string prefix)
        {
            var matches = movie.SymbolClasses.Where(symbol => symbol.Name.StartsWith(prefix, StringComparison.Ordinal)).Select(symbol => symbol.CharacterId).Distinct().ToList();
            return matches.Count == 1 ? matches[0] : throw new SwfFormatException($"{matches.Count} symbol classes start with '{prefix}'; exactly one is required.");
        }

        public bool IsSprite(int characterId) => movie.Characters.GetValueOrDefault(characterId) is SwfSprite;

        public int FrameCount(int spriteId) => Played(spriteId).FrameCount;

        /// <summary>The frames (1-based) of a sprite on which a text field sits directly on its display list; ffdec cannot draw those with the game's non-embedded fonts.</summary>
        public IReadOnlyList<int> FramesShowingText(int spriteId) =>
            Played(spriteId).Frames
                .Select((frame, index) => (Frame: index + 1, Shows: frame.Any(o => movie.Characters.GetValueOrDefault(o.CharacterId) is SwfEditText)))
                .Where(f => f.Shows).Select(f => f.Frame).ToList();

        /// <summary>The frame (1-based) a label names.</summary>
        public int FrameOfLabel(int spriteId, string label) =>
            Played(spriteId).Labels.TryGetValue(label, out int frame) ? frame : throw new SwfFormatException($"No frame of sprite {spriteId} is labelled '{label}'.");

        /// <summary>The only object of <paramref name="spriteId"/> that carries the instance name, in the state it is shown in.</summary>
        public SwfDisplayObject NamedChild(int spriteId, string instanceName) =>
            Single(Objects(spriteId).Where(o => o.Name == instanceName), $"instance '{instanceName}' of sprite {spriteId}");

        /// <summary>The only object of <paramref name="spriteId"/> without an instance name.</summary>
        public SwfDisplayObject OnlyUnnamedChild(int spriteId) =>
            Single(Objects(spriteId).Where(IsUnnamed), $"unnamed object of sprite {spriteId}");

        /// <summary>
        /// The unnamed shape or sprite of <paramref name="spriteId"/> that lies nearest to the object named <paramref name="siblingName"/>,
        /// measured between placement origins. A character placed more than once is a tile or background, not a glyph, and is not a
        /// candidate; the nearest candidate must be strictly nearer than every other.
        /// </summary>
        public SwfDisplayObject UnnamedBeside(int spriteId, string siblingName)
        {
            SwfDisplayObject sibling = NamedChild(spriteId, siblingName);
            var candidates = Objects(spriteId).Where(o => IsUnnamed(o) && movie.Characters.GetValueOrDefault(o.CharacterId) is SwfShape or SwfSprite)
                .GroupBy(o => o.CharacterId).Where(group => group.Count() == 1).Select(group => group.Single())
                .Select(o => (Object: o, Distance: Math.Sqrt(Math.Pow(o.Matrix.Tx - sibling.Matrix.Tx, 2) + Math.Pow(o.Matrix.Ty - sibling.Matrix.Ty, 2))))
                .OrderBy(candidate => candidate.Distance).ToList();
            string description = $"unnamed glyph beside '{siblingName}' in sprite {spriteId}";
            if (candidates.Count == 0)
                throw new SwfFormatException($"No {description}: the sprite places no unnamed shape or sprite once.");
            return candidates.Count == 1 || candidates[1].Distance > candidates[0].Distance
                ? candidates[0].Object
                : throw new SwfFormatException($"The {description} is ambiguous: two candidates are {candidates[0].Distance:0.##} twips away.");
        }

        static bool IsUnnamed(SwfDisplayObject o) => string.IsNullOrEmpty(o.Name);

        /// <summary>The only unnamed object that appears in the frame a label names.</summary>
        public SwfDisplayObject UnnamedChildAtLabel(int spriteId, string label)
        {
            int frame = FrameOfLabel(spriteId, label);
            var frames = Played(spriteId).Frames;
            var earlier = frame > 1 ? frames[frame - 2].Select(o => o.Life).ToHashSet() : [];
            var appearing = frames[frame - 1].Where(o => string.IsNullOrEmpty(o.Name) && !earlier.Contains(o.Life)).Select(o => o.Life).ToHashSet();
            return Single(Objects(spriteId).Where(o => appearing.Contains(o.Life)), $"unnamed placement in frame '{label}' of sprite {spriteId}");
        }

        /// <summary>
        /// Descends through single-frame wrappers that do nothing but host one sprite, to the sprite that actually animates;
        /// a sprite that is not such a wrapper is returned unchanged.
        /// </summary>
        public int AnimatedContent(int spriteId)
        {
            int current = spriteId;
            while (FrameCount(current) == 1)
            {
                var shown = Played(current).Frames[0];
                if (shown.Count != 1 || !IsSprite(shown[0].CharacterId)) break;
                current = shown[0].CharacterId;
            }
            return current;
        }

        /// <summary>
        /// The box covering everything the movie's main timeline ever shows, in twips and the stage's coordinates: the union over all frames
        /// of every shown object's drawn extent, nested sprites included. It deliberately ignores masks and colour transforms.
        /// </summary>
        public TwipRect ContentBounds() =>
            TimelineExtent(Timeline(null), []) ?? throw new SwfFormatException("The main timeline shows nothing.");

        /// <summary>
        /// Every object a sprite shows, by identity: an object kept over several frames is one object, in the state of its first frame.
        /// Objects that frames place anew at the same depth with the same character and placement are one object too.
        /// </summary>
        List<SwfDisplayObject> Objects(int spriteId)
        {
            var distinct = new List<SwfDisplayObject>();
            foreach (SwfDisplayObject first in Played(spriteId).Frames.SelectMany(frame => frame).GroupBy(o => o.Life).Select(states => states.First()))
                if (!distinct.Any(known => known.Depth == first.Depth && known.CharacterId == first.CharacterId && known.PlacementEquals(first)))
                    distinct.Add(first);
            return distinct;
        }

        /// <summary>
        /// The object a lookup selected, after checking that it keeps one placement through every frame it is shown in: a piece that is
        /// positioned by its placement has one placement, so an object that moves or changes between frames cannot stand for one.
        /// </summary>
        public SwfDisplayObject RequireStablePlacement(int spriteId, SwfDisplayObject chosen)
        {
            foreach (SwfDisplayObject state in Played(spriteId).Frames.SelectMany(frame => frame).Where(o => o.Life == chosen.Life))
                if (state.CharacterId != chosen.CharacterId || !state.PlacementEquals(chosen))
                    throw new SwfFormatException($"The object at depth {chosen.Depth} of sprite {spriteId} changes its placement between frames.");
            return chosen;
        }

        TwipRect? Extent(int characterId, HashSet<int> visiting)
        {
            switch (movie.Characters.GetValueOrDefault(characterId))
            {
                case SwfShape shape:
                    return shape.Bounds;
                case SwfEditText text:
                    return text.Bounds;
                case SwfSprite:
                    if (_extentCache.TryGetValue(characterId, out TwipRect? cached)) return cached;
                    if (!visiting.Add(characterId)) throw new SwfFormatException($"Sprite {characterId} contains itself.");
                    TwipRect? extent = TimelineExtent(Played(characterId), visiting);
                    visiting.Remove(characterId);
                    return _extentCache[characterId] = extent;
                default:
                    throw new SwfFormatException($"Character {characterId} is neither a shape, a text field nor a sprite, so its extent is unknown.");
            }
        }

        TwipRect? TimelineExtent(SwfPlayedTimeline timeline, HashSet<int> visiting)
        {
            TwipRect? union = null;
            foreach (SwfDisplayObject shown in timeline.Frames.SelectMany(frame => frame))
                if (Extent(shown.CharacterId, visiting) is { } extent)
                    union = union is { } rect ? rect.Union(extent.Transform(shown.Matrix)) : extent.Transform(shown.Matrix);
            return union;
        }

        static SwfDisplayObject Single(IEnumerable<SwfDisplayObject> candidates, string description)
        {
            var list = candidates.ToList();
            return list.Count == 1 ? list[0] : throw new SwfFormatException($"{list.Count} placements match {description}; exactly one is required.");
        }

        SwfPlayedTimeline Played(int spriteId) => Timeline(spriteId);

        /// <summary>A timeline as <see cref="SwfTimelinePlayer"/> plays it, played once per tree.</summary>
        /// <param name="spriteId"><c>null</c> for the main timeline.</param>
        SwfPlayedTimeline Timeline(int? spriteId)
        {
            int key = spriteId ?? -1;
            if (_played.TryGetValue(key, out SwfPlayedTimeline? played))
                return played;
            SwfTimelineTags tags = spriteId is { } id
                ? (movie.Characters.GetValueOrDefault(id) as SwfSprite)?.Timeline ?? throw new SwfFormatException($"Character {id} is not a sprite.")
                : movie.Root;
            return _played[key] = SwfTimelinePlayer.Play(tags, spriteId is null ? "the main timeline" : $"sprite {spriteId}");
        }
    }
}
