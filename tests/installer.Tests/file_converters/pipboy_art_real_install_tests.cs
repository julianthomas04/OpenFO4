using AssetConverter;
using Xunit;

namespace InstallerTests
{
    /// <summary>
    /// Checks against the Fallout 4 install named by <c>OPENFO4_FO4_PATH</c>: the Pip-Boy art catalog finds every symbol it names in
    /// the game's movies. Resolve only, so ffdec is not needed. Explicit: run with <c>-- xUnit.Explicit=only</c>; it reads the game's
    /// interface archive and so never runs in the regular suite.
    /// </summary>
    public sealed class PipboyArtRealInstallTests
    {
        private static string Game()
        {
            string? path = Environment.GetEnvironmentVariable("OPENFO4_FO4_PATH");
            Assert.SkipWhen(string.IsNullOrEmpty(path), "OPENFO4_FO4_PATH names no Fallout 4 install.");
            return path!;
        }

        [Fact(Explicit = true)]
        public void EverySymbolOfTheCatalog_IsFoundInItsMovie()
        {
            var movies = PipboyArtExtractor.ReadMovies(Game());

            var plan = PipboyArtPlan.Build(movies);
            var resolved = PipboyArtPlan.ResolveAll(movies, plan);

            Assert.Equal(plan.Symbols.Count, resolved.Count);
            Assert.All(resolved, clip => Assert.NotEmpty(clip.Symbol.Frames));
            Assert.Equal(plan.ClipIds.Count, plan.ClipIds.Distinct().Count());
        }

        [Fact(Explicit = true)]
        public void EveryMovieOfThePlan_IsInTheArchive()
        {
            var movies = PipboyArtExtractor.ReadMovies(Game());

            var plan = PipboyArtPlan.Build(movies);

            Assert.All(plan.Movies, movie => Assert.True(movies.ContainsKey(movie.Movie), movie.Movie));
        }
    }
}
