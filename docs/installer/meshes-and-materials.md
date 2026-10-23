# Meshes and materials

`NifConverter` converts every game mesh to GLB. It starts while the archives are
still being extracted.

The material census reads every material file once and is the single source of
what a material is, for the mesh converter, the palette baker and the database.
A mesh's material travels inside its GLB.

A texture index built from the archives' file lists lets the mesh converter
check that a texture exists before it is on disk.

Some materials store a grey texture and a separate colour palette.
`PaletteBaker` bakes the colour into the texture, so the engine gets an ordinary
coloured texture.
