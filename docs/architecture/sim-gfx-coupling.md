# Simulation / Rendering Coupling Analysis

Pre-work for libliero_sim (Sprint 3). Files that need decoupling from rendering before extracting the headless sim library.

## Files with rendering entanglement

| File | Depends on | Notes |
|------|-----------|-------|
| src/game/worm.cpp | gfx/renderer.hpp | For Bitmap type in draw calls |
| src/game/game.cpp | SDL.h, gfx/renderer.hpp | SDL random seed, Renderer param in focus() |
| src/game/weapon.cpp | gfx/renderer.hpp | Drawing laser sight |
| src/game/nobject.cpp | gfx/renderer.hpp | Drawing nobject trails |
| src/game/sobject.cpp | gfx/renderer.hpp | Drawing explosion sprites |
| src/game/bonus.cpp | Clean | No rendering dependencies ✅ |

## Decoupling strategy

1. Move all `draw*` methods out of simulation structs into renderer-side visitors
2. Replace `#include "gfx/renderer.hpp"` with forward declarations in simulation headers
3. game.cpp: separate SDL seed init from Game constructor
4. Create `src/sim/` directory with only simulation code
5. CMake target `liero_sim` (static lib) must compile with `-DLIERO_HEADLESS` to skip all rendering

## Target C ABI (sim_c_api.h)

```c
sim_t*    sim_create(void);
void      sim_destroy(sim_t*);
int       sim_load_tc(sim_t*, const char* path);
void      sim_step(sim_t*, frame_input[]);
void      sim_snapshot(sim_t*, snapshot_t*);
void      sim_rollback_to(sim_t*, int frame);
uint64_t  sim_checksum(sim_t*, int frame);
```
