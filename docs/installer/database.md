# Database

The database lane reads `Fallout4.esm`, the game's main data file, into a SQLite
database that the engine queries.

## Loading

`ESMLoader` opens the data file through Mutagen, a C# library for Bethesda data
files, and runs its record processors in parallel. Each record type has a
processor that writes a shared row in `records` plus that type's own tables.
`RawEsmScanner` reads the file's bytes directly for data Mutagen doesn't expose.

Names in the game data are ids into per-language string tables. The installer
extracts the installed language's tables to resolve them.

## Schema

`file_converters/esm_schema.sql` defines the database, and the engine's tests build against the
same file. Every record has a row in `records`, keyed by its form key, which the
domain tables reference. Nested data is stored as JSON.

## Finalization

Material swaps are resolved at install time into final texture paths, so the
engine only matches strings. Once the database and the converted assets both
exist, a final lane fills in what depends on both: colour-variant swaps, links
to distant low-detail models, distant water and normalized model paths.
