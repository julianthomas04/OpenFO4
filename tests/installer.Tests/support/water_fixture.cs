using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Noggog;

namespace InstallerTests
{
    /// <summary>A WATR record with every field set to a distinct, non-default value.</summary>
    internal static class WaterFixture
    {
        public const byte Opacity = 77;
        public const float NoiseOneWindDirection = 0.125f;
        public const string NoiseOneTexture = @"data\Textures\Water\Fixture.dds";
        public static readonly FormKey SoundKey = FormKey.Factory("0ABCDE:Fallout4.esm");
        public static readonly byte[] GnamBytes =
        [
            0x42, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00,
            0x18, 0x00, 0x00, 0x00,
        ];

        public static void Configure(Water water)
        {
            water.Name = "Fixture Water";
            water.Opacity = Opacity;
            water.Flags = Water.Flag.Dangerous | Water.Flag.DirectionalSound;
            water.OpenSound.FormKey = SoundKey;
            water.ConsumeSpell.FormKey = FormKey.Factory("0ABCDF:Fallout4.esm");
            water.ContactSpell.FormKey = FormKey.Factory("0ABCE0:Fallout4.esm");
            water.ImageSpace.FormKey = FormKey.Factory("0ABCE1:Fallout4.esm");
            water.Material.FormKey = FormKey.Factory("0ABCE2:Fallout4.esm");
            water.DATA = new byte[] { 1, 2 };
            water.GNAM = GnamBytes;
            water.LinearVelocity = new P3Float(0.5f, -0.25f, 0f);
            water.AngularVelocity = new P3Float(0.1f, 0.2f, 0.3f);
            water.DNAMDataTypeState = Water.DNAMDataType.Break0;

            water.FogDepthAmount = 1.5f;
            water.FogShallowColor = System.Drawing.Color.FromArgb(40, 10, 20, 30);
            water.FogDeepColor = System.Drawing.Color.FromArgb(41, 11, 21, 31);
            water.FogColorShallowRange = 2f;
            water.FogColorDeepRange = 3f;
            water.FogShallowAlpha = 0.4f;
            water.FogDeepAlpha = 0.6f;
            water.FogAlphaShallowRange = 4f;
            water.FogAlphaDeepRange = 5f;
            water.FogUnderwaterColor = System.Drawing.Color.FromArgb(42, 12, 22, 32);
            water.FogUnderwaterAmount = 6f;
            water.FogUnderwaterNear = 7f;
            water.FogUnderwaterFar = 8f;
            water.PhysicalNormalMagnitude = 0.11f;
            water.PhysicalShallowNormalFalloff = 0.12f;
            water.PhysicalDeepNormalFalloff = 0.13f;
            water.PhysicalReflectivityAmount = 0.14f;
            water.PhysicalFresnelAmount = 0.15f;
            water.PhysicalSurfaceEffectFalloff = 0.16f;
            water.PhysicalDisplacementSimulatorForce = 0.17f;
            water.PhysicalDisplacementSimulatorVelocity = 0.18f;
            water.PhysicalDisplacementSimulatorFalloff = 0.19f;
            water.PhysicalDisplacementSimulatorDampener = 0.2f;
            water.PhysicalDisplacementSimulatorStartingSize = 0.21f;
            water.PhysicalReflectionColor = System.Drawing.Color.FromArgb(43, 13, 23, 33);
            water.SpecularSunSpecularPower = 9f;
            water.SpecularSunSpecularMagnitude = 10f;
            water.SpecularSunSparklePower = 11f;
            water.SpecularSunSparkleMagnitude = 12f;
            water.SpecularInteriorSpecularRadius = 13f;
            water.SpecularInteriorSpecularBrightness = 14f;
            water.SpecularInteriorSpecularPower = 15f;
            water.SiltAmount = 0.5f;
            water.SiltLightColor = System.Drawing.Color.FromArgb(44, 14, 24, 34);
            water.SiltDarkColor = System.Drawing.Color.FromArgb(45, 15, 25, 35);
            water.ScreenSpaceReflections = true;
            ConfigureNoise(water.NoiseLayerOne, NoiseOneWindDirection, NoiseOneTexture);
            ConfigureNoise(water.NoiseLayerTwo, 0.25f, @"data\Textures\Water\Two.dds");
            ConfigureNoise(water.NoiseLayerThree, 0.5f, @"data\Textures\Water\Three.dds");
        }

        static void ConfigureNoise(WaterNoiseProperties noise, float windDirection, string texture)
        {
            noise.WindDirection = windDirection;
            noise.WindSpeed = windDirection + 1f;
            noise.AmplitudeScale = windDirection + 2f;
            noise.UvScale = windDirection + 3f;
            noise.NoiseFalloff = windDirection + 4f;
            noise.Texture = texture;
        }
    }
}
