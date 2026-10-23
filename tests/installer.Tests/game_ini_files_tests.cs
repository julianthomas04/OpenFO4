using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public sealed class GameIniFilesTests : IDisposable
    {
        readonly string _game = Directory.CreateTempSubdirectory("fo4-game-").FullName;
        readonly string _documents = Directory.CreateTempSubdirectory("fo4-docs-").FullName;

        public void Dispose()
        {
            Directory.Delete(_game, recursive: true);
            Directory.Delete(_documents, recursive: true);
        }

        static void Write(string directory, string name) => File.WriteAllText(Path.Combine(directory, name), "");

        [Fact]
        public void Sources_AreListedLowestPrecedenceFirstWithDefaultAndPrefsRequired()
        {
            var sources = GameIniFiles.Sources(_game, _documents);

            Assert.Equal(
                [
                    (Path.Combine(_game, "Fallout4_Default.ini"), true),
                    (Path.Combine(_documents, "Fallout4.ini"), false),
                    (Path.Combine(_documents, "Fallout4Prefs.ini"), true),
                    (Path.Combine(_documents, "Fallout4Custom.ini"), false),
                ],
                sources.Select(source => (source.Path, source.Required)));
        }

        [Fact]
        public void MissingRequired_NamesDefaultAndPrefsWhenNothingExists()
        {
            var missing = GameIniFiles.MissingRequired(GameIniFiles.Sources(_game, _documents));

            Assert.Equal(
                [Path.Combine(_game, "Fallout4_Default.ini"), Path.Combine(_documents, "Fallout4Prefs.ini")],
                missing);
        }

        [Fact]
        public void MissingRequired_NamesOnlyPrefsWhenTheLauncherNeverWroteIt()
        {
            Write(_game, "Fallout4_Default.ini");

            var missing = GameIniFiles.MissingRequired(GameIniFiles.Sources(_game, _documents));

            Assert.Equal([Path.Combine(_documents, "Fallout4Prefs.ini")], missing);
        }

        [Fact]
        public void MissingRequired_IgnoresAbsentOptionalFiles()
        {
            Write(_game, "Fallout4_Default.ini");
            Write(_documents, "Fallout4Prefs.ini");

            Assert.Empty(GameIniFiles.MissingRequired(GameIniFiles.Sources(_game, _documents)));
        }

        [Fact]
        public void MissingError_IsNullWhenTheRequiredFilesExist()
        {
            Write(_game, "Fallout4_Default.ini");
            Write(_documents, "Fallout4Prefs.ini");

            Assert.Null(GameIniFiles.MissingError(_game, _documents));
        }

        [Fact]
        public void MissingError_AsksToStartTheGameWhenOnlyPrefsIsMissing()
        {
            Write(_game, "Fallout4_Default.ini");

            Assert.Equal(
                $"Fallout 4 has not been started yet. Start Fallout 4 once (to the main menu) so it creates its settings files, then try again. If Fallout 4 keeps them somewhere else, point the Fallout 4 settings folder (--settings-dir) at the folder that holds them. Missing: {Path.Combine(_documents, "Fallout4Prefs.ini")}.",
                GameIniFiles.MissingError(_game, _documents));
        }

        [Fact]
        public void MissingError_BlamesTheInstallWhenTheShippedFileIsMissing()
        {
            Write(_documents, "Fallout4Prefs.ini");

            string? error = GameIniFiles.MissingError(_game, _documents);

            Assert.StartsWith("The Fallout 4 installation looks incomplete. Verify the game files in Steam", error);
            Assert.EndsWith($"Missing: {Path.Combine(_game, "Fallout4_Default.ini")}.", error);
            Assert.DoesNotContain("has not been started", error);
        }

        [Fact]
        public void MissingError_ReportsBothCausesWhenNothingExists()
        {
            string? error = GameIniFiles.MissingError(_game, _documents);

            Assert.Contains("looks incomplete", error);
            Assert.Contains("has not been started yet", error);
            Assert.Contains(Path.Combine(_game, "Fallout4_Default.ini"), error);
            Assert.Contains(Path.Combine(_documents, "Fallout4Prefs.ini"), error);
        }

        [Fact]
        public void MissingError_IgnoresAbsentOptionalFiles()
        {
            Write(_game, "Fallout4_Default.ini");
            Write(_documents, "Fallout4Prefs.ini");

            Assert.False(File.Exists(Path.Combine(_documents, "Fallout4.ini")));
            Assert.Null(GameIniFiles.MissingError(_game, _documents));
        }

        [Fact]
        public void Existing_ListsOnlyTheFilesPresentInPrecedenceOrder()
        {
            Write(_game, "Fallout4_Default.ini");
            Write(_documents, "Fallout4Custom.ini");

            var existing = GameIniFiles.Existing(GameIniFiles.Sources(_game, _documents));

            Assert.Equal(
                [Path.Combine(_game, "Fallout4_Default.ini"), Path.Combine(_documents, "Fallout4Custom.ini")],
                existing);
        }
    }
}
