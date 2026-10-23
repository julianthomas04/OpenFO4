# Reloaded.Memory 9.4.1 matching source

The preferred source ZIP preserves Reloaded.Memory revision 2c31a5e84aa06dbe8e22ed3859b79d8729618fb2 and includes its required Reloaded.Project.Configurations gitlink at revision 38db64b2a6c6952a66e1fe00cd344bc2f31c4b70.

The supplemental archive SHA256 is eb85c59134e4a17701c7c1186df13d82e308ce099f12d4bcd64e60c3a906882c. All 16 regular files are included beneath src/Reloaded.Project.Configurations, including NuGet.Build.props, Directory.Build.props, LICENSE.md and Reloaded-Icon.png. The src/.editorconfig symlink is replaced only with the exact included target bytes (SHA256 adbc5347cc41e36b7003a1b9b8b702c1da1c62b73ce0f3c178e3a125a4d4e40d).

Unpack the preferred ZIP and enter Reloaded.Memory-2c31a5e84aa06dbe8e22ed3859b79d8729618fb2. Preserve this directory layout. With .NET SDK 8.0.426 and the project's declared NuGet package inputs, build the library and its source generator with:

```sh
dotnet build src/Reloaded.Memory/Reloaded.Memory.csproj -c Release -f net8.0 --no-incremental -p:ImportDirectoryBuildProps=false -p:GeneratePackageOnBuild=false
```

`ImportDirectoryBuildProps=false` prevents the SDK from automatically importing an unrelated ancestor Directory.Build.props when this source is unpacked beneath another repository. The project's explicit NuGet.Build.props import still loads the included, pinned Reloaded.Project.Configurations/Directory.Build.props. Compiler warnings, analyzers, the source generator, source files and target framework are retained; no analyzer-disable property is used. `GeneratePackageOnBuild=false` limits this command to compilation instead of producing a NuGet package.

This command was verified with SDK 8.0.426 against preferred source SHA256 e9577e4551626d66d28caac3d4992a2b1fb4c8f52b4df72eab6d4084ad4f3ddf: the netstandard2.0 source generator and net8.0 library compiled with zero warnings and zero errors after forcing a non-incremental build. This proves source compilation, not byte-for-byte reproduction of the original NuGet binary.

The source-exclusion receipt inside the preferred ZIP records the exact upstream and supplemental archives and the single materialized link. Full GPL-3.0 text accompanies the submodule; the original project and its other notices remain intact.
