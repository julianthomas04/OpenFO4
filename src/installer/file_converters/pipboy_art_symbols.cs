using AssetConverter.Swf;

namespace AssetConverter
{
    /// <summary>A symbol found in a movie: its character, the timeline frames (1-based) a clip is made of, and how a doll piece is placed.</summary>
    internal sealed record ResolvedSymbol(int CharacterId, IReadOnlyList<int> Frames, IReadOnlyList<double>? Offset = null, IReadOnlyList<double>? Tint = null);

    /// <summary>Turns the catalog's name-based symbol sources into characters, frames and placements using the structure of the movie.</summary>
    internal static class PipboyArtSymbols
    {
        public static int Character(SymbolRef symbol, PipboySwfTree tree) => symbol switch
        {
            SymbolRef.ByClass byClass => tree.CharacterOfClass(byClass.ClassName),
            SymbolRef.ByClassPrefix byPrefix => tree.CharacterOfClassPrefix(byPrefix.Prefix),
            SymbolRef.NamedChild child => tree.NamedChild(Character(child.Parent, tree), child.InstanceName).CharacterId,
            SymbolRef.OnlyUnnamedChild child => tree.OnlyUnnamedChild(Character(child.Parent, tree)).CharacterId,
            SymbolRef.UnnamedAtLabel child => tree.UnnamedChildAtLabel(Character(child.Parent, tree), child.Label).CharacterId,
            SymbolRef.UnnamedBeside child => tree.UnnamedBeside(Character(child.Parent, tree), child.SiblingName).CharacterId,
            SymbolRef.AnimatedContentOf wrapper => tree.AnimatedContent(Character(wrapper.Wrapper, tree)),
            _ => throw new ArgumentOutOfRangeException(nameof(symbol), symbol, "Unknown symbol reference."),
        };

        public static ResolvedSymbol Sequence(NamedSymbolClipSource source, PipboySwfTree tree)
        {
            int character = Character(source.Symbol, tree);
            int available = tree.IsSprite(character) ? tree.FrameCount(character) : 1;
            IReadOnlyList<int> frames = source.Frames switch
            {
                FrameSelection.All => [.. Enumerable.Range(1, available)],
                FrameSelection.Only only => [only.Frame],
                FrameSelection.Labelled labelled => [LabelledFrame(source, character, labelled.Label, tree)],
                FrameSelection.UntilTrailingText => [.. Enumerable.Range(1, FirstTrailingTextFrame(source, character, tree, available) - 1)],
                _ => throw new ArgumentOutOfRangeException(nameof(source), source.Frames, "Unknown frame selection."),
            };
            if (frames.Count == 0 || frames[0] < 1 || frames[^1] > available)
                throw new SwfFormatException($"Clip '{source.ClipId}' wants frames {(frames.Count == 0 ? "none" : $"{frames[0]}..{frames[^1]}")} of a symbol with {available}.");
            return new ResolvedSymbol(character, frames);
        }

        static int LabelledFrame(NamedSymbolClipSource source, int character, string label, PipboySwfTree tree) =>
            tree.IsSprite(character)
                ? tree.FrameOfLabel(character, label)
                : throw new SwfFormatException($"Clip '{source.ClipId}' wants the frame labelled '{label}' of a symbol that has no timeline.");

        static int FirstTrailingTextFrame(NamedSymbolClipSource source, int character, PipboySwfTree tree, int last)
        {
            var textFrames = tree.IsSprite(character) ? tree.FramesShowingText(character).Where(f => f <= last).ToList() : [];
            if (textFrames.Count == 0) return last + 1;
            int first = textFrames[0];
            if (textFrames.Count != last - first + 1)
                throw new SwfFormatException($"Clip '{source.ClipId}': frames {first}..{last} are not all text frames, so the art cannot be cut before them.");
            return first;
        }

        /// <summary>
        /// The first frame of a doll piece with the offset and tint its placement in the doll gives it: the translation in pixels and the
        /// red, green, blue and alpha multipliers, exactly as the movie states them. A placement that scales, rotates or adds to a colour
        /// channel cannot be carried by the offset and multiplier contract and throws.
        /// </summary>
        public static ResolvedSymbol DollPiece(DollPieceSource source, PipboySwfTree tree)
        {
            if ((source.InstanceName is null) == (source.FrameLabel is null))
                throw new ArgumentException($"Doll piece '{source.ClipId}' needs exactly one of an instance name and a frame label.", nameof(source));

            int container = tree.CharacterOfClass(source.ContainerClass);
            SwfDisplayObject placement = tree.RequireStablePlacement(container, source.InstanceName is { } instance
                ? tree.NamedChild(container, instance)
                : tree.UnnamedChildAtLabel(container, source.FrameLabel!));

            if (!placement.Matrix.IsTranslationOnly)
                throw new SwfFormatException($"Doll piece '{source.ClipId}' is placed with a scale or rotation, which the offset-only contract cannot carry.");
            if (placement.Color.HasAddTerms)
                throw new SwfFormatException($"Doll piece '{source.ClipId}' is placed with a colour transform that adds to a channel, which the multiplier-only tint cannot carry.");
            double[] offset = [Twips.ToPixels(placement.Matrix.Tx), Twips.ToPixels(placement.Matrix.Ty)];
            double[]? tint = placement.Color.Multipliers is var multipliers && multipliers.Any(multiplier => multiplier != 1) ? multipliers : null;
            return new ResolvedSymbol(placement.CharacterId, [1], offset, tint);
        }
    }
}
