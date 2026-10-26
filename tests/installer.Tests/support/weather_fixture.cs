namespace InstallerTests
{
    /// <summary>A WTHR record whose DATA bytes and cloud layer are distinct and exactly representable after Mutagen's scaling.</summary>
    internal static class WeatherFixture
    {
        public const double WindSpeed = 51.0 / 255.0;
        public const double WindTurbulence = 102.0 / 255.0;
        public const double SunGlare = 153.0 / 255.0;
        public const double ThunderFrequency = 204.0 / 255.0;

        public const byte WindDirectionRaw = 100;
        public const byte WindDirectionRangeRaw = 45;
        public const byte TransDeltaRaw = 10;
        public const float WindDirection = WindDirectionRaw / 360f;
        public const float WindDirectionRange = WindDirectionRangeRaw / 180f;
        public const float TransDelta = TransDeltaRaw * 4f;

        /// <summary>The stored sound types and their names, in the order the fixture weather authors its sounds.</summary>
        public static readonly (uint Raw, string Kind)[] SoundKinds =
        [
            (0, "default"),
            (1, "precipitation"),
            (2, "wind"),
            (3, "thunder"),
        ];

        public const string ParticleMaterial = @"effects\FXWeatherRaindrops.BGEM";
        public const string ParticleTexture = @"effects\weather\raindrop_d.dds";

        public const int ShortDataLength = 19;

        /// <summary>A 20 byte DATA: wind speed 255, transition delta 25, thunder frequency 255, wind direction 180, range 90, turbulence 127.</summary>
        public static readonly byte[] RawData =
        [
            255, 0, 0, 25, 0, 0, 0, 0, 0, 0,
            255, 0, 0, 0, 0, 0, 0, 180, 90, 127,
        ];

        public const int LightningRed = 250;
        public const float VolatilityMult = 1.5f;

        public const int TexturedCloudSlot = 5;
        public const string CloudTexture = @"Sky\Clouds\Fixture05.dds";
        public const int ConfiguredCloudLayer = 3;
        public const float CloudXSpeed = 0.05f;
        public const float CloudYSpeed = -0.05f;
        public const int CloudDayRed = 200;
        public const float CloudDayAlpha = 0.5f;
        public const float CloudEarlySunriseAlpha = 0.25f;
    }
}
