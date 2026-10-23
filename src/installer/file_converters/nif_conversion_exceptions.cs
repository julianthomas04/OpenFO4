namespace AssetConverter
{
    public class NifAnimationException : Exception
    {
        public NifAnimationException(string message) : base(message) { }
        public NifAnimationException(string message, Exception inner) : base(message, inner) { }
    }

    internal sealed class NonFiniteParticleValueException : NifAnimationException
    {
        public NonFiniteParticleValueException(float value) : base($"non-finite particle value {value}") { }
    }

    public class NifSkinException : Exception
    {
        public NifSkinException(string message) : base(message) { }
    }

    public class NifConnectPointException : Exception
    {
        public NifConnectPointException(string message) : base(message) { }
    }
}
