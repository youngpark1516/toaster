# Rendering notes

A **ray** has an origin and direction. The camera sends primary rays through image pixels; intersections create new rays that scatter according to surface materials.

Each pixel uses multiple random **samples** to estimate incoming light. More samples reduce noise. A path stops after its configured maximum **bounces** (or later through probabilistic termination). Sample colors are **accumulated** in linear space before averaging and tonemapping for output.

A **BVH** groups nearby geometry in nested bounding boxes. Testing boxes before primitives avoids most expensive intersection work and becomes essential for triangle meshes.

