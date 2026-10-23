using System.Globalization;
using System.Text.RegularExpressions;
using AssetConverter.Abc;
using AssetConverter.Swf;

namespace AssetConverter
{
    /// <summary>A movie read from the interface archive: its path below <c>Interface\</c> and its bytes.</summary>
    internal sealed record ArtMovie(string Source, byte[] Content);

    /// <summary>One clip of a symbol and how to find the symbol in its movie.</summary>
    internal sealed record SymbolWork(string ClipId, string Movie, string? Illustrates, ArtColour Colour, Func<PipboySwfTree, ResolvedSymbol> Resolve);

    internal sealed record ResolvedClip(SymbolWork Work, ResolvedSymbol Symbol);

    /// <summary>
    /// Every clip the export makes: the symbol clips with the way each symbol is found, and the clips of whole movies. Built from the
    /// catalog and from what the movies themselves state (the damage type count, the map marker classes, the perk card symbols, the
    /// condition figures and Vault Boy movies the archive holds).
    /// </summary>
    internal sealed partial record PipboyArtPlan(IReadOnlyList<SymbolWork> Symbols, IReadOnlyList<MovieClipSource> Movies)
    {
        public IReadOnlyList<string> ClipIds => [.. Symbols.Select(work => work.ClipId), .. Movies.Select(movie => movie.ClipId)];

        [GeneratedRegex(@"^PerkClip[0-9a-f]+$")]
        private static partial Regex PerkCardClass();

        public static PipboyArtPlan Build(IReadOnlyDictionary<string, ArtMovie> movies)
        {
            var symbols = new List<SymbolWork>();
            symbols.AddRange(PipboyArtCatalog.Icons.Select(Named));
            symbols.AddRange(PipboyArtCatalog.DollPieces.Select(piece =>
                new SymbolWork(piece.ClipId, piece.Movie, null, ArtColour.Mask, tree => PipboyArtSymbols.DollPiece(piece, tree))));
            ArtMovie inventory = Require(movies, PipboyArtCatalog.InventoryPage);
            int damageTypeCount = DamageTypeCount(inventory);
            PipboyArtCatalog.RequireMatchingTypeCount(damageTypeCount);
            symbols.AddRange(PipboyArtCatalog.NamedSymbols.Concat(PipboyArtCatalog.PerkChartSymbols).Concat(PipboyArtCatalog.InterfaceSymbols)
                .Concat(PipboyArtCatalog.DamageIcons(damageTypeCount)).Select(Named));
            symbols.AddRange(PipboyArtCatalog.MarkerSources(MarkerClasses(Require(movies, PipboyArtCatalog.MapPage))).Select(marker =>
                new SymbolWork(marker.ClipId, marker.Movie, null, ArtColour.Mask, tree => new ResolvedSymbol(tree.CharacterOfClass(marker.ClassName), [1]))));
            symbols.AddRange(PerkCards(Require(movies, PipboyArtCatalog.LevelUpMenu)));

            var wholeMovies = new List<MovieClipSource>(PipboyArtCatalog.SpecialMovies);
            wholeMovies.AddRange(PipboyArtCatalog.ConditionMovies(movies.Keys));
            wholeMovies.AddRange(movies.Values.OrderBy(movie => movie.Source, StringComparer.OrdinalIgnoreCase)
                .Where(movie => PipboyArtCatalog.VaultBoyFolders.Any(folder => movie.Source.StartsWith(folder, StringComparison.OrdinalIgnoreCase)))
                .Select(movie => new MovieClipSource(PipboyArtCatalog.VaultBoyClipId(movie.Source), movie.Source)));

            foreach (string source in symbols.Select(s => s.Movie).Concat(wholeMovies.Select(m => m.Movie)).Distinct(StringComparer.OrdinalIgnoreCase))
                Require(movies, source);
            return new PipboyArtPlan(symbols, wholeMovies);
        }

        static SymbolWork Named(NamedSymbolClipSource source) =>
            new(source.ClipId, source.Movie, null, source.Colour, tree => PipboyArtSymbols.Sequence(source, tree));

        public static ArtMovie Require(IReadOnlyDictionary<string, ArtMovie> movies, string source) =>
            movies.TryGetValue(source, out ArtMovie? movie)
                ? movie
                : throw new PipboyArtException($"Movie '{source}' is missing from the interface archive.");

        static T Guard<T>(ArtMovie movie, Func<T> work) =>
            ConversionGuard.Run(movie.Source, work, (message, cause) => new PipboyArtException(message, cause));

        /// <summary>The damage type count the inventory page's scripts state, which places each damage glyph after its resistance glyph.</summary>
        static int DamageTypeCount(ArtMovie inventory)
        {
            AbcConstantMap scripts = ScriptsOf(inventory);
            return scripts.Constants.GetValueOrDefault(PipboyArtCatalog.DamageTypeCountConstant) is int count and > 0
                ? count
                : throw new PipboyArtException($"'{inventory.Source}' states no positive integer {PipboyArtCatalog.DamageTypeCountConstant}"
                    + (scripts.Unresolved.TryGetValue(PipboyArtCatalog.DamageTypeCountConstant, out string? why) ? $" ({why})." : "."));
        }

        /// <summary>The classes of the map's marker types in type order, as the map's scripts list them.</summary>
        static IReadOnlyList<string> MarkerClasses(ArtMovie map)
        {
            AbcConstantMap scripts = ScriptsOf(map);
            return scripts.ReferenceLists.TryGetValue(PipboyArtCatalog.MarkerListMember, out IReadOnlyList<string>? classes) && classes.Count > 0
                ? classes
                : throw new PipboyArtException($"'{map.Source}' lists no marker classes in {PipboyArtCatalog.MarkerListMember}"
                    + (scripts.Unresolved.TryGetValue(PipboyArtCatalog.MarkerListMember, out string? why) ? $" ({why})." : "."));
        }

        static AbcConstantMap ScriptsOf(ArtMovie movie) => Guard(movie, () => AbcConstantExtractor.Extract(SwfMovieReader.Read(movie.Content), movie.Source));

        static List<SymbolWork> PerkCards(ArtMovie levelUpMenu) => Guard(levelUpMenu, () =>
        {
            SwfMovie parsed = SwfMovieReader.Read(levelUpMenu.Content);
            var cards = parsed.SymbolClasses.Select(symbol => symbol.Name).Where(name => PerkCardClass().IsMatch(name))
                .Distinct().Order(StringComparer.Ordinal).ToList();
            if (cards.Count == 0)
                throw new PipboyArtException($"'{levelUpMenu.Source}' exports no perk card symbols.");
            return cards.Select((card, index) => new SymbolWork(
                string.Create(CultureInfo.InvariantCulture, $"perkcard/{index}"), PipboyArtCatalog.LevelUpMenu, PipboyArtCatalog.PerkMoviePath(card), ArtColour.Full, cardTree =>
                {
                    int characterId = cardTree.CharacterOfClass(card);
                    return cardTree.IsSprite(characterId)
                        ? new ResolvedSymbol(characterId, Enumerable.Range(1, cardTree.FrameCount(characterId)).ToList())
                        : throw new SwfFormatException($"Perk card symbol '{card}' is not a sprite.");
                })).ToList();
        });

        /// <summary>The symbols of one movie, found through its structure.</summary>
        public static IReadOnlyList<ResolvedClip> Resolve(ArtMovie movie, PipboySwfTree tree, IEnumerable<SymbolWork> works) =>
            [.. works.Select(work => new ResolvedClip(work, ResolveSymbol(work, tree, movie)))];

        static ResolvedSymbol ResolveSymbol(SymbolWork work, PipboySwfTree tree, ArtMovie movie)
        {
            try
            {
                return work.Resolve(tree);
            }
            catch (SwfFormatException failure)
            {
                throw new PipboyArtException($"Clip '{work.ClipId}': symbol of '{movie.Source}' not found: {failure.Message}", failure);
            }
        }

        /// <summary>Finds every symbol of the plan in its movie, without exporting anything.</summary>
        public static IReadOnlyList<ResolvedClip> ResolveAll(IReadOnlyDictionary<string, ArtMovie> movies, PipboyArtPlan plan) =>
            [.. plan.Symbols.GroupBy(work => work.Movie, StringComparer.OrdinalIgnoreCase).SelectMany(group =>
            {
                ArtMovie movie = Require(movies, group.Key);
                return Guard(movie, () => Resolve(movie, new PipboySwfTree(SwfMovieReader.Read(movie.Content)), group));
            })];
    }
}
