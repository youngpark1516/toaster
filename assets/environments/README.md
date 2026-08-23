# Environment assets

## Qwantani Sunset (Pure Sky)

- Source asset: **Qwantani Sunset (Pure Sky)** (`qwantani_sunset_puresky`)
- Source page: https://polyhaven.com/a/qwantani_sunset_puresky
- Original 2K HDR: https://dl.polyhaven.org/file/ph-assets/HDRIs/hdr/2k/qwantani_sunset_puresky_2k.hdr
- Authors: Greg Zaal (photography), Jarod Guest (processing)
- License: CC0 1.0 — https://polyhaven.com/license
- Retrieved: 2026-08-12
- Original file: `qwantani_sunset_puresky_2k_source.hdr`
- Original SHA-256: `d66e08231e9c09ca40c6d035214ae8efb638782745c82addbb2a063fa32cabef`
- Derived scene file: `sunset_room_2k.hdr`
- Derived SHA-256: `ca0312a876682477a5c1e979e79406c35b8b7d265382043f7a0812839734f1ac`

### Linear HDR color grade

The derived map was decoded to linear `RGB32F`, graded per pixel, and encoded
directly back to Radiance HDR without an LDR intermediate or output clamp.
For source values `R`, `G`, and `B`:

```text
Y = 0.2126 R + 0.7152 G + 0.0722 B
w = clamp(0.5 + 1.25 (R - B) / max(Y, 1e-6), 0, 1)

R' = R (1.03 + 0.19 w)
G' = G (1.00 + 0.02 w)
B' = B (0.98 - 0.14 w)
```

The conversion used `image` 0.25's Radiance decoder and encoder in a temporary,
uncommitted Rust helper. Validation statistics from the decoded pixels:

| Map | Maximum channel | Pixels above 1.0 | Blue-dominant pixels | Mean R-B |
| --- | ---: | ---: | ---: | ---: |
| Original | 125440.000000 | 610450 | 1869805 | 0.146765 |
| Derived | 153036.796875 | 619111 | 1838616 | 0.331796 |

Both maps contained 2,097,152 finite, nonnegative pixels. The derivative keeps
the unclipped high-radiance sun and substantial cooler upper-sky content while
increasing red/orange warmth.
