using Avalonia.Controls;
using Avalonia.Platform.Storage;

namespace AssetConverter.InstallerUi
{
    /// <summary>The platform's folder dialog, opened over <paramref name="owner"/>.</summary>
    internal sealed class StorageFolderPicker(TopLevel owner) : IFolderPicker
    {
        public async Task<string?> PickFolderAsync(string title, string? startFolder)
        {
            var storage = owner.StorageProvider;
            var options = new FolderPickerOpenOptions
            {
                Title = title,
                AllowMultiple = false,
                SuggestedStartLocation = startFolder != null ? await storage.TryGetFolderFromPathAsync(startFolder).ConfigureAwait(true) : null,
            };
            var picked = await storage.OpenFolderPickerAsync(options).ConfigureAwait(true);
            return picked.Count > 0 ? picked[0].TryGetLocalPath() : null;
        }
    }
}
