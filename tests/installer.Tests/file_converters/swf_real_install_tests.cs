using AssetConverter;
using AssetConverter.Swf;
using Xunit;

namespace InstallerTests
{
    /// <summary>
    /// Evidence from the Fallout 4 install named by <c>OPENFO4_FO4_PATH</c> for the reader's placement rules. Explicit: run with
    /// <c>-- xUnit.Explicit=only</c>; it reads the game's interface archive and so never runs in the regular suite.
    /// </summary>
    public sealed class SwfRealInstallTests
    {
        /// <summary>
        /// <c>StreamingInstallMenu.swf</c> places an image character with a PlaceObject3 whose image flag and character flag are both set.
        /// The file format specification says such a placement carries a class name; the file's bytes do not, and the reader (which fails
        /// on a placement whose flags leave bytes unexplained) reads the movie only because it takes the class name for an image
        /// placement without a character, as the Ruffle player's reader does.
        /// </summary>
        [Fact(Explicit = true)]
        public void StreamingInstallMenu_ImagePlacementWithACharacter_CarriesNoClassName()
        {
            string? game = Environment.GetEnvironmentVariable("OPENFO4_FO4_PATH");
            Assert.SkipWhen(string.IsNullOrEmpty(game), "OPENFO4_FO4_PATH names no Fallout 4 install.");

            var entry = InterfaceArchive.ReadMatching(game!, path => path.EndsWith(@"Interface\StreamingInstallMenu.swf", StringComparison.OrdinalIgnoreCase)).Single();

            SwfMovie movie = SwfMovieReader.Read(entry.Content);

            Assert.NotEmpty(movie.Characters);
        }
    }
}
