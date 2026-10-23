using Mutagen.Bethesda.Fallout4;

namespace InstallerTests
{
    /// <summary>A GRAS record with every DATA field set to a distinct, non-default value.</summary>
    internal static class GrassFixture
    {
        public const byte Density = 37;
        public const byte MinSlope = 5;
        public const byte MaxSlope = 65;
        public const ushort UnitsFromWater = 321;
        public const uint UnitsFromWaterType = 4;
        public const float PositionRange = 12.5f;
        public const float HeightRange = 0.25f;
        public const float ColorRange = 0.75f;
        public const float WavePeriod = 3.5f;
        public const int FlagsRaw = 0x1 | 0x4;

        public static void ConfigureFirst(Grass grass)
        {
            grass.Density = Density;
            grass.MinSlope = MinSlope;
            grass.MaxSlope = MaxSlope;
            grass.UnitsFromWater = UnitsFromWater;
            grass.UnitsFromWaterType = (Grass.UnitsFromWaterTypeEnum)UnitsFromWaterType;
            grass.PositionRange = PositionRange;
            grass.HeightRange = HeightRange;
            grass.ColorRange = ColorRange;
            grass.WavePeriod = WavePeriod;
            grass.Flags = (Grass.Flag)FlagsRaw;
        }
    }
}
