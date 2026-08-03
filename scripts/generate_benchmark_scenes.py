#!/usr/bin/env python3
"""Generate Toaster's deterministic geometry-scaling benchmark fixtures."""

from __future__ import annotations

import base64
import json
import math
import struct
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parent.parent
SCENE_DIRECTORY = REPOSITORY_ROOT / "scenes" / "benchmarks"
ASSET_DIRECTORY = REPOSITORY_ROOT / "assets" / "benchmarks"

RENDER_SETTINGS = {
    "width": 320,
    "height": 180,
    "samples": 4,
    "max_bounces": 4,
    "background": "black",
}

CAMERA = {
    "position": [0.0, 5.5, 8.0],
    "look_at": [0.0, 0.0, 0.0],
    "up": [0.0, 1.0, 0.0],
    "fov_degrees": 45.0,
}

MATERIALS = [
    {"name": "surface", "type": "diffuse", "albedo": [0.62, 0.68, 0.74]},
    {"name": "accent", "type": "metal", "albedo": [0.82, 0.48, 0.2], "roughness": 0.2},
    {"name": "light", "type": "emissive", "color": [1.0, 0.92, 0.78], "strength": 14.0},
]


def write_json(path: Path, value: object) -> None:
    """Write stable, readable JSON with one final newline."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def terrain_height(x: float, z: float) -> float:
    """Return the shared low-amplitude terrain height."""
    return 0.16 * math.sin(0.9 * x) * math.cos(0.9 * z)


def terrain_normal(x: float, z: float) -> tuple[float, float, float]:
    """Return the analytic unit normal of ``terrain_height``."""
    derivative_x = 0.144 * math.cos(0.9 * x) * math.cos(0.9 * z)
    derivative_z = -0.144 * math.sin(0.9 * x) * math.sin(0.9 * z)
    normal = (-derivative_x, 1.0, -derivative_z)
    length = math.sqrt(sum(component * component for component in normal))
    return tuple(component / length for component in normal)


def generate_grid_gltf(subdivisions: int) -> None:
    """Generate an embedded-buffer glTF terrain with two triangles per cell."""
    extent = 4.0
    positions: list[float] = []
    normals: list[float] = []
    for row in range(subdivisions + 1):
        z = -extent + 2.0 * extent * row / subdivisions
        for column in range(subdivisions + 1):
            x = -extent + 2.0 * extent * column / subdivisions
            positions.extend((x, terrain_height(x, z), z))
            normals.extend(terrain_normal(x, z))

    indices: list[int] = []
    row_width = subdivisions + 1
    for row in range(subdivisions):
        for column in range(subdivisions):
            lower_left = row * row_width + column
            lower_right = lower_left + 1
            upper_left = lower_left + row_width
            upper_right = upper_left + 1
            indices.extend((lower_left, upper_left, lower_right))
            indices.extend((lower_right, upper_left, upper_right))

    if max(indices) > 65535:
        raise ValueError("benchmark grid exceeds unsigned-short glTF indices")

    position_bytes = struct.pack(f"<{len(positions)}f", *positions)
    normal_bytes = struct.pack(f"<{len(normals)}f", *normals)
    index_bytes = struct.pack(f"<{len(indices)}H", *indices)
    normal_offset = len(position_bytes)
    index_offset = normal_offset + len(normal_bytes)
    payload = position_bytes + normal_bytes + index_bytes

    gltf = {
        "asset": {"version": "2.0", "generator": "Toaster benchmark fixture generator"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [
            {
                "name": f"BenchmarkTerrain{subdivisions}",
                "primitives": [
                    {
                        "attributes": {"POSITION": 0, "NORMAL": 1},
                        "indices": 2,
                        "mode": 4,
                    }
                ],
            }
        ],
        "accessors": [
            {
                "bufferView": 0,
                "componentType": 5126,
                "count": len(positions) // 3,
                "type": "VEC3",
                "min": [-extent, -0.16, -extent],
                "max": [extent, 0.16, extent],
            },
            {
                "bufferView": 1,
                "componentType": 5126,
                "count": len(normals) // 3,
                "type": "VEC3",
            },
            {
                "bufferView": 2,
                "componentType": 5123,
                "count": len(indices),
                "type": "SCALAR",
            },
        ],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": len(position_bytes), "target": 34962},
            {
                "buffer": 0,
                "byteOffset": normal_offset,
                "byteLength": len(normal_bytes),
                "target": 34962,
            },
            {
                "buffer": 0,
                "byteOffset": index_offset,
                "byteLength": len(index_bytes),
                "target": 34963,
            },
        ],
        "buffers": [
            {
                "byteLength": len(payload),
                "uri": "data:application/octet-stream;base64,"
                + base64.b64encode(payload).decode("ascii"),
            }
        ],
    }
    write_json(ASSET_DIRECTORY / f"terrain_grid_{subdivisions}.gltf", gltf)


def base_scene(objects: list[dict[str, object]]) -> dict[str, object]:
    """Create a benchmark scene with shared camera, quality, and materials."""
    return {
        "camera": CAMERA,
        "render": RENDER_SETTINGS,
        "materials": MATERIALS,
        "objects": objects,
    }


def light() -> dict[str, object]:
    """Return the common emissive sphere used by geometry workloads."""
    return {
        "type": "sphere",
        "center": [0.0, 4.5, 0.5],
        "radius": 0.65,
        "material": "light",
    }


def terrain_scene(subdivisions: int) -> dict[str, object]:
    """Create a triangle-scaling scene using one generated terrain mesh."""
    return base_scene(
        [
            {
                "type": "mesh",
                "path": f"../../assets/benchmarks/terrain_grid_{subdivisions}.gltf",
                "material": "surface",
            },
            light(),
        ]
    )


def sphere_objects(count: int, material: str = "surface") -> list[dict[str, object]]:
    """Lay out exactly ``count`` non-overlapping spheres over a square footprint."""
    columns = math.ceil(math.sqrt(count))
    rows = math.ceil(count / columns)
    spacing_x = 7.2 / max(columns - 1, 1)
    spacing_z = 7.2 / max(rows - 1, 1)
    radius = min(spacing_x, spacing_z) * 0.34
    objects: list[dict[str, object]] = []
    for index in range(count):
        column = index % columns
        row = index // columns
        x = -3.6 + column * spacing_x
        z = -3.6 + row * spacing_z
        y = radius + 0.08 * math.sin(index * 0.73)
        objects.append(
            {
                "type": "sphere",
                "center": [round(x, 7), round(y, 7), round(z, 7)],
                "radius": round(radius, 7),
                "material": material if index % 5 else "accent",
            }
        )
    return objects


def ground_triangles() -> list[dict[str, object]]:
    """Return a fixed two-triangle ground plane for sphere-only workloads."""
    return [
        {
            "type": "triangle",
            "vertices": [[-5.0, 0.0, -5.0], [-5.0, 0.0, 5.0], [5.0, 0.0, -5.0]],
            "material": "surface",
        },
        {
            "type": "triangle",
            "vertices": [[5.0, 0.0, -5.0], [-5.0, 0.0, 5.0], [5.0, 0.0, 5.0]],
            "material": "surface",
        },
    ]


def sphere_scene(count: int) -> dict[str, object]:
    """Create a sphere-scaling scene with fixed ground and lighting."""
    return base_scene(ground_triangles() + sphere_objects(count) + [light()])


def mixed_scene() -> dict[str, object]:
    """Create a representative terrain-plus-spheres traversal workload."""
    objects: list[dict[str, object]] = [
        {
            "type": "mesh",
            "path": "../../assets/benchmarks/terrain_grid_32.gltf",
            "material": "surface",
        }
    ]
    for sphere in sphere_objects(128):
        center = sphere["center"]
        assert isinstance(center, list)
        center[1] = round(float(center[1]) + 0.22, 7)
        sphere["radius"] = round(float(sphere["radius"]) * 0.72, 7)
        objects.append(sphere)
    objects.append(light())
    return base_scene(objects)


def environment_control_scene() -> dict[str, object]:
    """Create a low-geometry control for non-BVH environment-lighting costs."""
    scene = base_scene(
        [
            {"type": "sphere", "center": [-1.2, 0.7, 0.0], "radius": 0.7, "material": "accent"},
            {"type": "sphere", "center": [0.0, 0.7, -0.2], "radius": 0.7, "material": "surface"},
            {"type": "sphere", "center": [1.2, 0.7, 0.0], "radius": 0.7, "material": "accent"},
            {"type": "sphere", "center": [0.0, -1000.0, 0.0], "radius": 1000.0, "material": "surface"},
        ]
    )
    scene["camera"] = {
        "position": [0.0, 1.25, 5.5],
        "look_at": [0.0, 0.65, 0.0],
        "up": [0.0, 1.0, 0.0],
        "fov_degrees": 42.0,
    }
    render = dict(RENDER_SETTINGS)
    render["background"] = {
        "type": "environment",
        "path": "../../assets/environments/studio_test.hdr",
        "intensity": 8.0,
        "rotation_degrees": 20.0,
    }
    scene["render"] = render
    return scene


def main() -> None:
    """Regenerate every benchmark asset and scene in deterministic order."""
    for subdivisions in (8, 32, 64):
        generate_grid_gltf(subdivisions)

    scenes = {
        "001_triangles_128.json": terrain_scene(8),
        "002_triangles_2048.json": terrain_scene(32),
        "003_triangles_8192.json": terrain_scene(64),
        "004_spheres_64.json": sphere_scene(64),
        "005_spheres_512.json": sphere_scene(512),
        "006_mixed_2048t_128s.json": mixed_scene(),
        "007_environment_control.json": environment_control_scene(),
    }
    for name, scene in scenes.items():
        write_json(SCENE_DIRECTORY / name, scene)


if __name__ == "__main__":
    main()
