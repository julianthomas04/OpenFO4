using AssetConverter;
using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Installation;
using Xunit;

namespace InstallerTests.ShaderTranslation
{
    public class ShaderOutputLockTests
    {
        private static string Stem(TempAssetRoot root) => Path.Combine(root.Path, InstallLayout.ShadersFolder);

        [Fact]
        public void Acquire_CreatesTheLockFileNextToTheShadersFolderAndRemovesItOnDispose()
        {
            using var root = new TempAssetRoot();
            string stem = Stem(root);

            var held = ShaderOutputLock.Acquire(stem);

            Assert.Equal(stem + ".lock", held.LockPath);
            Assert.True(File.Exists(held.LockPath));
            held.Dispose();
            Assert.False(File.Exists(held.LockPath));
        }

        [Fact]
        public void Acquire_WhileAnotherRunHoldsTheLock_ThrowsTheTypedErrorNamingTheLockFile()
        {
            using var root = new TempAssetRoot();
            string stem = Stem(root);
            using var firstRun = ShaderOutputLock.Acquire(stem);

            var ex = Assert.Throws<ShaderOutputLockedException>(() => ShaderOutputLock.Acquire(stem));

            Assert.Equal(firstRun.LockPath, ex.LockPath);
            Assert.Contains(firstRun.LockPath, ex.Message);
            Assert.IsAssignableFrom<IOException>(ex.InnerException);
        }

        [Fact]
        public void Acquire_RefusedAttempt_DoesNotDisturbTheHolder()
        {
            using var root = new TempAssetRoot();
            string stem = Stem(root);
            using var firstRun = ShaderOutputLock.Acquire(stem);
            Assert.Throws<ShaderOutputLockedException>(() => ShaderOutputLock.Acquire(stem));

            Assert.True(File.Exists(firstRun.LockPath));
            Assert.Throws<ShaderOutputLockedException>(() => ShaderOutputLock.Acquire(stem));
        }

        [Fact]
        public void Acquire_AfterTheHolderDisposed_Succeeds()
        {
            using var root = new TempAssetRoot();
            string stem = Stem(root);
            ShaderOutputLock.Acquire(stem).Dispose();

            using var nextRun = ShaderOutputLock.Acquire(stem);

            Assert.True(File.Exists(nextRun.LockPath));
        }

        [Fact]
        public void Acquire_LockFileWithoutAnyHolder_IsTakenOverAndRemovedAfterwards()
        {
            using var root = new TempAssetRoot();
            string stem = Stem(root);
            string leftover = ShaderOutputLock.PathFor(stem);
            File.WriteAllText(leftover, "written by a run that was killed");

            ShaderOutputLock.Acquire(stem).Dispose();

            Assert.False(File.Exists(leftover));
        }

        [Fact]
        public void Acquire_DifferentAssetRoots_DoNotBlockEachOther()
        {
            using var first = new TempAssetRoot();
            using var second = new TempAssetRoot();
            using var firstRun = ShaderOutputLock.Acquire(Stem(first));

            using var secondRun = ShaderOutputLock.Acquire(Stem(second));

            Assert.NotEqual(firstRun.LockPath, secondRun.LockPath);
        }

        [Fact]
        public void Acquire_LockPathIsADirectory_ThrowsStepErrorNotTheContentionError()
        {
            using var root = new TempAssetRoot();
            string stem = Stem(root);
            Directory.CreateDirectory(ShaderOutputLock.PathFor(stem));

            var ex = Assert.Throws<ShaderTranslateStepException>(() => ShaderOutputLock.Acquire(stem));

            Assert.Contains(ShaderOutputLock.PathFor(stem), ex.Message);
        }

        [Fact]
        public void Acquire_ParentDirectoryMissing_ThrowsStepErrorNotTheContentionError()
        {
            using var root = new TempAssetRoot();

            var ex = Assert.Throws<ShaderTranslateStepException>(() => ShaderOutputLock.Acquire(Path.Combine(root.Path, "missing", InstallLayout.ShadersFolder)));

            Assert.IsType<DirectoryNotFoundException>(ex.InnerException);
        }
    }
}
