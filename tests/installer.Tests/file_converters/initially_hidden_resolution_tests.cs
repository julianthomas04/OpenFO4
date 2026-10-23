using AssetConverter;
using Xunit;

namespace InstallerTests
{
    // ESMLoader.ResolveInitiallyHiddenAsync over hand-written placed_objects rows (the real schema): the XESP
    // enable-chain verdict that becomes the engine's spawn filter. Typical chains are covered end to end in
    // EsmLoadPlacedObjectTests; these are the structural edge cases.
    public sealed class InitiallyHiddenResolutionTests : IDisposable
    {
        readonly SchemaDatabase _db = SchemaDatabase.CreateAsync().GetAwaiter().GetResult();

        public void Dispose() => _db.Dispose();

        void AddRef(string formKey, bool disabled = false, string? parent = null, bool opposite = false) =>
            _db.Execute(
                "INSERT INTO placed_objects (form_key, base_object, object_type, is_initially_disabled, enable_parent, enable_parent_opposite) VALUES (@formKey, '000001:Base.esm', 'PlacedObject', @disabled, @parent, @opposite)",
                new { formKey, disabled = disabled ? 1 : 0, parent, opposite = opposite ? 1 : 0 });

        bool IsHidden(string formKey) =>
            _db.QuerySingle<long>("SELECT initially_hidden FROM placed_objects WHERE form_key = @formKey", new { formKey }) == 1;

        long HiddenCount() => _db.QuerySingle<long>("SELECT COUNT(*) FROM placed_objects WHERE initially_hidden = 1");

        Task Resolve() => ESMLoader.ResolveInitiallyHiddenAsync(_db.Connection, null);

        [Fact]
        public async Task Resolve_ParentMissingFromTable_CountsAsEnabled()
        {
            AddRef("000001:A.esm", parent: "0000FF:Gone.esm");
            AddRef("000002:A.esm", parent: "0000FF:Gone.esm", opposite: true);

            await Resolve();

            Assert.False(IsHidden("000001:A.esm"));
            Assert.True(IsHidden("000002:A.esm"));
        }

        [Fact]
        public async Task Resolve_VeryLongChainFromDisabledRoot_HidesEveryLinkWithoutOverflowing()
        {
            AddRef("000000:A.esm", disabled: true);
            for (int i = 1; i < 5000; i++)
                AddRef($"{i:X6}:A.esm", parent: $"{i - 1:X6}:A.esm");

            await Resolve();

            Assert.Equal(5000, HiddenCount());
        }

        [Fact]
        public async Task Resolve_ChainWithAlternatingOppositeFlags_AlternatesVisibility()
        {
            AddRef("000000:A.esm");
            for (int i = 1; i <= 6; i++)
                AddRef($"{i:X6}:A.esm", parent: $"{i - 1:X6}:A.esm", opposite: true);

            await Resolve();

            Assert.Equal(["000001:A.esm", "000003:A.esm", "000005:A.esm"],
                _db.Query<string>("SELECT form_key FROM placed_objects WHERE initially_hidden = 1 ORDER BY form_key"));
        }

        [Fact]
        public async Task Resolve_ManyHiddenRefs_MarksAllOfThem()
        {
            for (int i = 0; i < 1300; i++)
                AddRef($"{i:X6}:A.esm", disabled: true);
            AddRef("FFFFFF:A.esm");

            await Resolve();

            Assert.Equal(1300, HiddenCount());
            Assert.False(IsHidden("FFFFFF:A.esm"));
        }

        [Fact]
        public async Task Resolve_RefsWithoutAnyDisabledState_StayVisible()
        {
            AddRef("000001:A.esm");
            AddRef("000002:A.esm", parent: "000001:A.esm");

            await Resolve();

            Assert.Equal(0, HiddenCount());
        }

        [Fact]
        public async Task Resolve_EmptyTable_DoesNothing()
        {
            await Resolve();

            Assert.Equal(0, HiddenCount());
        }

        [Fact]
        public async Task Resolve_InsideCallersTransaction_IsRolledBackWithIt()
        {
            AddRef("000001:A.esm", disabled: true);

            using (var transaction = _db.Connection.BeginTransaction())
            {
                await ESMLoader.ResolveInitiallyHiddenAsync(_db.Connection, transaction);
                transaction.Rollback();
            }

            Assert.Equal(0, HiddenCount());
        }

        [Fact]
        public async Task Resolve_InsideCallersTransaction_CommitsWithIt()
        {
            AddRef("000001:A.esm", disabled: true);

            using (var transaction = _db.Connection.BeginTransaction())
            {
                await ESMLoader.ResolveInitiallyHiddenAsync(_db.Connection, transaction);
                transaction.Commit();
            }

            Assert.Equal(1, HiddenCount());
        }
    }
}
