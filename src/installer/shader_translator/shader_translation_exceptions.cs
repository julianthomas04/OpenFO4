namespace AssetConverter.ShaderTranslation
{
    public class ShaderTranslationException : Exception
    {
        public ShaderTranslationException(string message) : base(message) { }
        public ShaderTranslationException(string message, Exception inner) : base(message, inner) { }
    }

    public sealed class ShaderArchiveException : ShaderTranslationException
    {
        public ShaderArchiveException(string message) : base(message) { }
        public ShaderArchiveException(string message, Exception inner) : base(message, inner) { }
    }

    public sealed class FxpFormatException : ShaderTranslationException
    {
        public long Offset { get; }

        public FxpFormatException(long offset, string message) : base($"FXP offset 0x{offset:X}: {message}")
        {
            Offset = offset;
        }
    }

    public sealed class DxbcFormatException : ShaderTranslationException
    {
        public DxbcFormatException(string message) : base(message) { }
    }

    public sealed class Sm5DecodeException : ShaderTranslationException
    {
        public int TokenOffset { get; }

        public Sm5DecodeException(int tokenOffset, string message) : base($"SM5 token offset {tokenOffset}: {message}")
        {
            TokenOffset = tokenOffset;
        }
    }

    public sealed class WgslUnsupportedException : ShaderTranslationException
    {
        public WgslUnsupportedException(string message) : base(message) { }
        public WgslUnsupportedException(string message, Exception inner) : base(message, inner) { }
    }

    /// <summary>Another shader-translate run owns the shaders folder; nothing was changed.</summary>
    public sealed class ShaderOutputLockedException : ShaderTranslationException
    {
        public string LockPath { get; }

        public ShaderOutputLockedException(string lockPath, Exception inner)
            : base($"Another shader-translate run is writing the shaders folder ({lockPath}). Wait for it to finish and run shader-translate again; the lock is released when that run ends, even if its process was killed.", inner)
        {
            LockPath = lockPath;
        }
    }

    public sealed class UnmappedResourceException : ShaderTranslationException
    {
        public UnmappedResourceException(string message) : base(message) { }
    }
}
