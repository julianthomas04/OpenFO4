using AssetConverter;
using AssetConverter.Swf;

namespace InstallerTests
{
    /// <summary>
    /// The movies of a miniature game interface archive that hold what the Pip-Boy art plan reads: the damage type count, the map's
    /// marker class list, perk card symbols and the condition figures. Names and numbers are placeholders.
    /// </summary>
    internal static class SyntheticArtArchive
    {
        public static byte[] Plain() => new SwfMovieBuilder().Build();

        public static byte[] WithDamageTypes(int count)
        {
            var abc = new AbcBuilder();
            abc.Class("Shared", "GlobalFunc", 0, abc.Method(), abc.Method(), classTraits: [new AbcSlotSpec(abc.Name("NUM_DAMAGE_TYPES"), true, abc.Int(count), AbcBuilder.ConstantInt)]);
            return new SwfMovieBuilder().Abc("inventory", abc.Build()).Build();
        }

        public static byte[] WithMarkerClasses(params string[] classes)
        {
            var abc = new AbcBuilder();
            var code = new AbcCode().GetLocal0().PushScope().FindProperty(abc.Name("MarkerTypesA"));
            foreach (string name in classes)
                code.GetLex(abc.Name(name));
            code.Op(0x56, classes.Length).InitProperty(abc.Name("MarkerTypesA"));
            int cinit = abc.Function(code.ReturnVoid().ToArray());
            abc.Class("", "MapHolder", 0, abc.Method(), cinit, classTraits: [new AbcSlotSpec(abc.Name("MarkerTypesA"))]);
            return new SwfMovieBuilder().Abc("map", abc.Build()).Build();
        }

        public static byte[] WithPerkCards(params string[] classes)
        {
            var swf = new SwfMovieBuilder();
            var symbols = classes.Select((name, index) => (CharacterId: index + 1, Name: name)).Append((90, "SampleMenu")).ToArray();
            foreach ((int id, _) in symbols)
                swf.Sprite(id, t => t.ShowFrame());
            return swf.SymbolClass(symbols).Build();
        }

        public static Dictionary<string, ArtMovie> Movies(int damageTypes = 6, string[]? markers = null, int bodies = 3, params string[] extra)
        {
            var movies = new Dictionary<string, ArtMovie>(StringComparer.OrdinalIgnoreCase);
            void Add(string source, byte[] content) => movies[source] = new ArtMovie(source, content);

            foreach (string page in new[] { PipboyArtCatalog.StatsPage, PipboyArtCatalog.DataPage, PipboyArtCatalog.Shell, PipboyArtCatalog.LoadingScreen })
                Add(page, Plain());
            Add(PipboyArtCatalog.InventoryPage, WithDamageTypes(damageTypes));
            Add(PipboyArtCatalog.MapPage, WithMarkerClasses(markers ?? ["FirstMarker", "SecondMarker"]));
            Add(PipboyArtCatalog.LevelUpMenu, WithPerkCards("PerkClip2b", "PerkClip0a", "Unrelated"));
            foreach (var special in PipboyArtCatalog.SpecialMovies)
                Add(special.Movie, Plain());
            Add(PipboyArtCatalog.ConditionHeadMovie, Plain());
            for (int body = 0; body < bodies; body++)
                Add($@"Components\ConditionClips\Condition_Body_{body}.swf", Plain());
            foreach (string path in extra)
                Add(path, Plain());
            return movies;
        }
    }
}
