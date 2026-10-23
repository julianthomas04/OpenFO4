# Characters and animation

This covers everything with a skeleton that plays animation: animated objects
such as doors and containers, and skinned models whose bones deform the mesh.

## Animation data

`anim/` holds the CPU side of animation as pure data and maths: skeletons,
clips of keyframes, a sampler that reads a clip at a given time, a player that
tracks time, speed and looping, and the resulting poses. Animation is read from
the converted glTF files, and clips are matched to a skeleton's joints by name.

Clips carry markers, named points in time that the player reports as it passes
them. This is how, for example, a door knows it has finished opening.

## Animated objects

Animated objects (`rendering/animated_objects.rs`) are updated only while a clip
is playing. A finished object keeps its pose and costs nothing per frame.

## Skinning

Skinning happens on the GPU. Each vertex records which bones move it and by how
much; every frame all bone transforms go into one shared buffer, and a shader
moves the vertices. Skinned models use the same material, lighting and shadow
paths as static objects.

## Ragdolls

A ragdoll-capable object draws as a static mesh until the
[simulation](simulation.md) publishes a pose for it. From then on a skinned copy,
posed by the simulation, replaces it. `ragdoll_sim_link.rs` is the single
connection between the two.
