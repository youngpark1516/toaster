# Roadmap

This file tracks milestones. Current behavior is documented from the [documentation index](README.md); future physics entries here are not implemented APIs.

## Phase 1: CPU reference renderer

Complete: rays, cameras, spheres, diffuse/metal/dielectric scattering, sampling, accumulation, and PNG output.

## Phase 1.5: Emission and Cornell box

Complete: emissive area lights, triangle intersections, indirect illumination,
and Cornell-box color bleeding.

## Phase 2: GPU renderer

Port sphere tracing to a `wgpu` compute shader, add device setup and image readback, and compare results against the CPU renderer.

## Phase 3: Meshes and BVH

Complete: triangles, median-split CPU BVH construction, flat CPU/GPU traversal,
and deterministic pre/post integration benchmark workloads.

## Phase 4: Assets and procedural scenes

Initial glTF/GLB import now includes triangle geometry, node transforms, smooth
normals, UVs, base-color factors and textures, relative scene paths, material
overrides, animation groups, and equirectangular HDR/PNG/JPEG environment
lighting. Luminance-weighted environment sampling and diffuse multiple
importance sampling now reduce noise around bright map regions on both CPU and
GPU. Next, add reusable procedural scene generators and expand PBR inputs.

## Phase 5: Preview server

The browser-facing service provides latest-frame MJPEG transport, render status,
adaptive samples per frame, and opt-in static progressive GPU accumulation.
Animated frames remain independent so different scene poses are never mixed.
