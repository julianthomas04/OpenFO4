using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public class LaunchArgumentsTests
    {
        private static Func<string, string?> Environment(params (string Name, string Value)[] variables) =>
            name => variables.Where(v => v.Name == name).Select(v => v.Value).FirstOrDefault();

        [Fact]
        public void WithoutFlagsOrVariablesNothingIsOn()
        {
            Assert.Equal(new LaunchArguments(Headless: false, KeepStaging: false), LaunchArguments.Parse([], Environment()));
        }

        [Fact]
        public void TheFlagsTurnTheirSwitchOnIndependently()
        {
            Assert.Equal(new LaunchArguments(Headless: true, KeepStaging: false), LaunchArguments.Parse(["--headless"], Environment()));
            Assert.Equal(new LaunchArguments(Headless: false, KeepStaging: true), LaunchArguments.Parse(["--keep-staging"], Environment()));
            Assert.Equal(new LaunchArguments(Headless: true, KeepStaging: true), LaunchArguments.Parse(["--keep-staging", "--headless"], Environment()));
        }

        [Fact]
        public void TheVariablesSetToOneTurnTheirSwitchOn()
        {
            var parsed = LaunchArguments.Parse([], Environment(("OPENFO4_HEADLESS", "1"), ("OPENFO4_KEEP_STAGING", "1")));

            Assert.Equal(new LaunchArguments(Headless: true, KeepStaging: true), parsed);
        }

        [Fact]
        public void TheSettingsFolderComesFromTheFlagOrElseTheVariable()
        {
            var variable = Environment(("OPENFO4_SETTINGS_DIR", @"E:\FromVariable"));

            Assert.Equal(@"D:\Settings", LaunchArguments.Parse(["--settings-dir", @"D:\Settings"], variable).SettingsDirectory);
            Assert.Equal(@"D:\Settings", LaunchArguments.Parse([@"--settings-dir=D:\Settings"], variable).SettingsDirectory);
            Assert.Equal(@"E:\FromVariable", LaunchArguments.Parse([], variable).SettingsDirectory);
            Assert.Null(LaunchArguments.Parse([], Environment()).SettingsDirectory);
        }

        [Fact]
        public void ASettingsFlagWithoutAValueIsRefused()
        {
            Assert.Throws<LaunchArgumentException>(() => LaunchArguments.Parse(["--settings-dir"], Environment()));
        }

        [Theory]
        [InlineData("0")]
        [InlineData("")]
        [InlineData("true")]
        public void AVariableWithAnyOtherValueLeavesTheSwitchOff(string value)
        {
            var parsed = LaunchArguments.Parse([], Environment(("OPENFO4_KEEP_STAGING", value)));

            Assert.False(parsed.KeepStaging);
        }
    }
}
