using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins.Binary.Parameters;
using Mutagen.Bethesda.Strings;

namespace AssetConverter
{
    public partial class ESMLoader
    {
        const int PluginHeaderFlagsOffset = 8;
        const uint LocalizedFlag = 0x80;

        private IFallout4ModDisposableGetter OpenPlugin(string path)
        {
            if (!IsLocalized(path))
                return Fallout4Mod.CreateFromBinaryOverlay(path, Fallout4Release.Fallout4);

            if (Strings == null)
                throw new InvalidOperationException($"'{path}' is a localized plugin but no string tables were supplied.");
            StringTables.Require(Strings, "Fallout4 - Interface.ba2");
            return Fallout4Mod.CreateFromBinaryOverlay(path, Fallout4Release.Fallout4,
                new BinaryReadParameters
                {
                    StringsParam = new StringsReadParameters { StringsFolderOverride = Strings.Folder, TargetLanguage = Strings.PluginLanguage },
                });
        }

        static bool IsLocalized(string path)
        {
            Span<byte> header = stackalloc byte[PluginHeaderFlagsOffset + sizeof(uint)];
            using var stream = File.OpenRead(path);
            stream.ReadExactly(header);
            return (BitConverter.ToUInt32(header[PluginHeaderFlagsOffset..]) & LocalizedFlag) != 0;
        }
    }
}
