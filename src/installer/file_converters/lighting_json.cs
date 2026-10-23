using Mutagen.Bethesda.Fallout4;

namespace AssetConverter
{
    // Plain-data projections of cell lighting (XCLL) and lighting templates. Colours are
    // {R,G,B}, and the property names are the ones the engine's RawLighting reads
    // (see rendering/lighting/cell_lighting.rs).
    public static class LightingJson
    {
        public static object? Project(ICellLightingGetter? l) => l == null ? null : new
        {
            AmbientColor = WeatherProcessor.Rgb(l.AmbientColor),
            DirectionalColor = WeatherProcessor.Rgb(l.DirectionalColor),
            FogNearColor = WeatherProcessor.Rgb(l.FogNearColor),
            FogFarColor = WeatherProcessor.Rgb(l.FogFarColor),
            l.FogNear,
            l.FogFar,
            l.DirectionalRotationXY,
            l.DirectionalRotationZ,
            l.DirectionalFade,
            l.FogClipDistance,
            l.FogPower,
            l.FogMax,
            AmbientColors = AmbientCube(l.AmbientColors),
            Inherits = (uint)l.Inherits,
            l.LightFadeBegin,
            l.LightFadeEnd,
            l.NearHeightMid,
            l.NearHeightRange,
            l.FarHeightMid,
            l.FarHeightRange,
            FogHighNearColor = WeatherProcessor.Rgb(l.ForColorHighNear),
            FogHighFarColor = WeatherProcessor.Rgb(l.ForColorHighFar),
            l.HighDensityScale,
            l.FogNearScale,
            l.FogFarScale,
            l.FogHighNearScale,
            l.FogHighFarScale
        };

        public static object Project(ILightingTemplateGetter l) => new
        {
            AmbientColor = WeatherProcessor.Rgb(l.AmbientColor),
            DirectionalColor = WeatherProcessor.Rgb(l.DirectionalColor),
            FogNearColor = WeatherProcessor.Rgb(l.FogNearColor),
            FogFarColor = WeatherProcessor.Rgb(l.FogFarColor),
            l.FogNear,
            l.FogFar,
            l.DirectionalRotationXY,
            l.DirectionalRotationZ,
            l.DirectionalFade,
            l.FogClipDistance,
            l.FogPower,
            l.FogMax,
            DirectionalAmbientColors = AmbientCube(l.DirectionalAmbientColors),
            l.LightFadeStartDistance,
            l.LightFadeEndDistance,
            l.NearHeightMid,
            l.NearHeightRange,
            l.FogHeightMid,
            l.FogHeightRange,
            FogColorHighNear = WeatherProcessor.Rgb(l.FogColorHighNear),
            FogColorHighFar = WeatherProcessor.Rgb(l.FogColorHighFar),
            l.HighDensityScale,
            l.FogNearScale,
            l.FogFarScale,
            l.FogHighNearScale,
            l.FogHighFarScale
        };

        private static object? AmbientCube(IAmbientColorsGetter? a) => a == null ? null : new
        {
            DirectionalXPlus = WeatherProcessor.Rgb(a.DirectionalXPlus),
            DirectionalXMinus = WeatherProcessor.Rgb(a.DirectionalXMinus),
            DirectionalYPlus = WeatherProcessor.Rgb(a.DirectionalYPlus),
            DirectionalYMinus = WeatherProcessor.Rgb(a.DirectionalYMinus),
            DirectionalZPlus = WeatherProcessor.Rgb(a.DirectionalZPlus),
            DirectionalZMinus = WeatherProcessor.Rgb(a.DirectionalZMinus),
            Specular = WeatherProcessor.Rgb(a.Specular),
            a.Scale
        };
    }
}
