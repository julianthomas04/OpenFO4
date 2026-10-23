namespace AssetConverter.Swf
{
    /// <summary>
    /// The state of one object on a display list at one frame. <see cref="Life"/> identifies the object across frames: it changes when a
    /// character is placed at the depth anew (also when a move replaces the character), and stays while later placements only modify it.
    /// </summary>
    internal sealed record SwfDisplayObject(
        int Life,
        int Depth,
        int CharacterId,
        string? Name,
        SwfMatrix Matrix,
        ColorTransform Color,
        int? Ratio,
        int? ClipDepth,
        IReadOnlyList<SwfFilter> Filters,
        SwfBlendMode BlendMode,
        bool CacheAsBitmap,
        bool Visible,
        string? ClassName,
        SwfColor? Background)
    {
        public bool IsMask => ClipDepth.HasValue;

        /// <summary>Whether two states of the same object differ in anything a frame shows.</summary>
        public bool PlacementEquals(SwfDisplayObject other) =>
            Matrix == other.Matrix && Color == other.Color && Ratio == other.Ratio && ClipDepth == other.ClipDepth
            && Filters.SequenceEqual(other.Filters) && BlendMode == other.BlendMode && CacheAsBitmap == other.CacheAsBitmap
            && Visible == other.Visible && Name == other.Name && Background == other.Background;
    }

    /// <summary>A timeline played into one display list per frame (frame 1 first), sorted by depth, with its labels.</summary>
    internal sealed record SwfPlayedTimeline(IReadOnlyList<IReadOnlyList<SwfDisplayObject>> Frames, IReadOnlyDictionary<string, int> Labels)
    {
        public int FrameCount => Frames.Count;
    }

    /// <summary>
    /// Plays the control tags of a timeline into display lists: a placement with a character introduces it at a depth (or, with the move
    /// flag, replaces the character there), one without a character modifies the object at the depth, a removal clears the depth.
    /// Anything that leaves the state a guess throws <see cref="SwfFormatException"/>.
    /// </summary>
    internal static class SwfTimelinePlayer
    {
        public static SwfPlayedTimeline Play(SwfTimelineTags timeline, string where)
        {
            var displayList = new SortedDictionary<int, SwfDisplayObject>();
            var frames = new List<IReadOnlyList<SwfDisplayObject>>(timeline.Frames.Count);
            var labels = new Dictionary<string, int>(StringComparer.Ordinal);
            int nextLife = 0;
            foreach (SwfFrame frame in timeline.Frames)
            {
                foreach (string label in frame.Labels)
                    if (!labels.TryAdd(label, frames.Count + 1))
                        throw new SwfFormatException($"Frame label '{label}' appears twice in {where}.");
                foreach (object action in frame.Actions)
                {
                    switch (action)
                    {
                        case SwfPlaceObject place:
                            Place(displayList, place, ref nextLife, where);
                            break;
                        case SwfRemoveObject remove:
                            displayList.Remove(remove.Depth);
                            break;
                    }
                }
                frames.Add(displayList.Values.ToList());
            }
            return new SwfPlayedTimeline(frames, labels);
        }

        private static void Place(SortedDictionary<int, SwfDisplayObject> displayList, SwfPlaceObject place, ref int nextLife, string where)
        {
            bool occupied = displayList.TryGetValue(place.Depth, out SwfDisplayObject? existing);
            if (place.CharacterId is null && existing is null)
                throw new SwfFormatException($"A placement in {where} modifies depth {place.Depth}, which holds nothing.");
            if (place.CharacterId is not null && occupied && !place.Move)
                throw new SwfFormatException($"A placement in {where} puts a character on depth {place.Depth} without a move, but the depth is occupied.");

            SwfDisplayObject current = existing ?? new SwfDisplayObject(
                -1, place.Depth, 0, null, SwfMatrix.Identity, ColorTransform.Identity, null, null, [], SwfBlendMode.Normal, false, true, null, null);
            displayList[place.Depth] = current with
            {
                Life = place.CharacterId is not null ? nextLife++ : current.Life,
                CharacterId = place.CharacterId ?? current.CharacterId,
                Name = place.Name ?? current.Name,
                Matrix = place.Matrix ?? current.Matrix,
                Color = place.Color ?? current.Color,
                Ratio = place.Ratio ?? current.Ratio,
                ClipDepth = place.ClipDepth ?? current.ClipDepth,
                Filters = place.Filters ?? current.Filters,
                BlendMode = place.BlendMode ?? current.BlendMode,
                CacheAsBitmap = place.CacheAsBitmap ?? current.CacheAsBitmap,
                Visible = place.Visible ?? current.Visible,
                ClassName = place.ClassName ?? current.ClassName,
                Background = place.Background ?? current.Background,
            };
        }
    }
}
