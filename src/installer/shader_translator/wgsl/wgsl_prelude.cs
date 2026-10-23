namespace AssetConverter.ShaderTranslation.Wgsl
{
    /// <summary>
    /// Module-scope text the composing wrapper prepends once before all translated fragments. Directives are only legal before the first
    /// declaration of a module, so they cannot be part of a per-shader fragment.
    /// </summary>
    public static class WgslPrelude
    {
        /// <summary>
        /// D3D allows derivatives and implicit-LOD sampling in divergent control flow (the values are merely undefined for inactive
        /// lanes); WGSL validators that enforce uniformity (Tint, future naga) would reject translated shaders without this directive.
        /// </summary>
        public const string DerivativeUniformityOff = "diagnostic(off, derivative_uniformity);";

        public const string Text = DerivativeUniformityOff + "\n";
    }
}
