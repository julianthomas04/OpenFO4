// Modified for OpenFO4 in 2026, including provenance notices; GPL-3.0-only. See SOURCE.md.
using System;
using System.IO;
using ICSharpCode.SharpZipLib.Zip.Compression;

namespace SharpBSABA2
{
    public class SharedExtractParams : IDisposable
    {
        public Inflater Inflater { get; private set; }
        public BinaryReader Reader { get; private set; }

        /// <param name="reader">True if a new <see cref="BinaryReader"/> should be created.</param>
        /// <param name="inflater">True if a new <see cref="Inflater"/> should be created.</param>
        public SharedExtractParams(Archive archive, bool reader, bool inflater)
        {
            this.Reader = reader
                ? new BinaryReader(new FileStream(archive.FullPath, FileMode.Open, FileAccess.Read), archive.Encoding)
                : archive.BinaryReader;

            this.Inflater = inflater ? new Inflater() : archive.Inflater;
        }

        public void Dispose()
        {
            ((IDisposable)Reader).Dispose();
            GC.SuppressFinalize(this);
        }
    }
}
