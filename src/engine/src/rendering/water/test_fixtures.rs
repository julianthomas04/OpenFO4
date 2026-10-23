//! Test support: a synthetic WATR record the register and shader tests start from.

use crate::assets::db::waters::WaterData;

/// A made-up water with round, mutually distinct values, in the shape the
/// installer writes into `data.db`.
pub const SYNTHETIC_WATER: &str = r#"{"Opacity":60,"Flags":[],"FogDepthAmount":2000,"FogShallowColor":[10,20,30,0],"FogDeepColor":[40,50,60,0],"FogColorShallowRange":0.5,"FogColorDeepRange":0.75,"FogShallowAlpha":0,"FogDeepAlpha":1,"FogAlphaShallowRange":1,"FogAlphaDeepRange":0.25,"FogUnderwaterColor":[100,110,120,0],"FogUnderwaterAmount":0.5,"FogUnderwaterNear":-5000,"FogUnderwaterFar":2000,"PhysicalNormalMagnitude":0.5,"PhysicalShallowNormalFalloff":0.9,"PhysicalDeepNormalFalloff":0.8,"PhysicalReflectivityAmount":0.3,"PhysicalFresnelAmount":0.1,"PhysicalSurfaceEffectFalloff":0.7,"PhysicalDisplacementSimulatorForce":0.2,"PhysicalDisplacementSimulatorVelocity":0.9,"PhysicalDisplacementSimulatorFalloff":0.7,"PhysicalDisplacementSimulatorDampener":0.95,"PhysicalDisplacementSimulatorStartingSize":0.1,"PhysicalReflectionColor":[50,60,70,0],"SpecularSunSpecularPower":2000,"SpecularSunSpecularMagnitude":10,"SpecularSunSparklePower":10000,"SpecularSunSparkleMagnitude":100,"SpecularInteriorSpecularRadius":5000,"SpecularInteriorSpecularBrightness":5,"SpecularInteriorSpecularPower":200,"SiltAmount":0.5,"SiltLightColor":[100,120,140,0],"SiltDarkColor":[20,30,40,0],"ScreenSpaceReflections":true,"LinearVelocity":[0,0,0],"AngularVelocity":[0,0,0],"NoiseLayerOne":{"WindDirection":45,"WindSpeed":0.01,"AmplitudeScale":0.5,"UvScale":8000,"NoiseFalloff":5000,"Texture":"data\\Textures\\Test\\NoiseA.dds"},"NoiseLayerTwo":{"WindDirection":90,"WindSpeed":0.02,"AmplitudeScale":0.6,"UvScale":500,"NoiseFalloff":3000,"Texture":"data\\Textures\\Test\\NoiseB.dds"},"NoiseLayerThree":{"WindDirection":135,"WindSpeed":0.03,"AmplitudeScale":0.7,"UvScale":200,"NoiseFalloff":2000,"Texture":"data\\Textures\\Test\\NoiseB.dds"}}"#;

pub fn synthetic_water() -> WaterData {
    WaterData::parse(SYNTHETIC_WATER).expect("the synthetic water fixture parses")
}
