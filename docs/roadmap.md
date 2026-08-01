# Roadmap

## Phase 1: CPU reference renderer

Complete: rays, cameras, spheres, diffuse/metal/dielectric scattering, sampling, accumulation, and PNG output.

## Phase 1.5: Emission and Cornell box

Add emissive area lights and initial triangle intersections, then demonstrate indirect illumination and color bleeding in a Cornell box. Triangle support remains a simple linear scan until BVH work begins.

## Phase 2: GPU renderer

Port sphere tracing to a `wgpu` compute shader, add device setup and image readback, and compare results against the CPU renderer.

## Phase 3: Meshes and BVH

Add triangles, construct a CPU BVH, flatten it into GPU-friendly buffers, and measure traversal performance.

## Phase 4: Assets and procedural scenes

Initial glTF/GLB import now includes triangle geometry, node transforms, smooth
normals, UVs, base-color factors and textures, relative scene paths, material
overrides, animation groups, and equirectangular HDR/PNG/JPEG environment
lighting. Next, add importance sampling for bright environment regions, then
reusable procedural scene generators.

## Phase 5: Preview server

Serve render status and images through a small browser-facing service without turning Toaster into a full editor.
