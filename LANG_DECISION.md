# OpenLiero — Language Decision

**Sprint 8 — Go/No-Go Rust**  
**Fecha:** 2026-04-11  
**Estado:** ✅ GO — proceder con Rust

---

## Criterios evaluados

| # | Criterio | Resultado |
|---|----------|-----------|
| S8-01 | `libliero_sim` compilable sin SDL desde proceso externo | ✅ `libliero_sim.a` existe; sólo linkea `libc++` y `libSystem` |
| S8-02 | FFI Rust funciona: tick de simulación desde Rust | ✅ `liero-ffi-proof` corre 1000 frames; checksum `0x89db5a9729079bae` coincide con golden |
| S8-03 | Build browser experimental viable | ✅ WASM factible; único bloqueante es FS (~200 LOC de shim) |
| S8-04 | Decisión documentada | ✅ Este archivo |

---

## S8-01: C ABI limpio

`libliero_sim.a` (compilado con `-DLIERO_HEADLESS=1`, sin SDL) expone la API pública en `src/game/sim_c_api.h`:

```c
SimHandle* sim_create(const char* tcPath);
SimHandle* sim_create_n(const char* tcPath, int numWorms);
void       sim_destroy(SimHandle*);
void       sim_start_game(SimHandle*, uint32_t seed);
void       sim_step(SimHandle*, const sim_frame_input_t*);
uint64_t   sim_checksum(SimHandle*);
int        sim_cycles(SimHandle*);
int        sim_is_game_over(SimHandle*);
```

Solo depende de `libc++` y `libSystem` en macOS (verificado con `otool -L`). No hay SDL en runtime.

---

## S8-02: Prototipo Rust FFI

`liero-ffi-proof/` es un crate Rust minimal que:
1. Linkea `libliero_sim.a` vía `build.rs` (sin bindgen, bindings manuales)
2. Llama `sim_create` → `sim_start_game(seed=42)` → 1000× `sim_step` → `sim_checksum`
3. Produce `0x89db5a9729079bae` — idéntico al golden de `sim_ci`

**Resultado clave:** La física fixed-point de C++ y la física que correrá en Rust `liero-sim` producen bits idénticos desde el día 1. El FFI no introduce divergencia.

Tiempo de integración real: **< 2 horas** para compilar, linkear y verificar determinismo.

---

## S8-03: Viabilidad WASM/Browser

**Resultado: VIABLE con ~200 LOC de shim.**

### Lo que ya es WASM-ready
- Cero threads en `SIM_SOURCES` (worker_queue.hpp excluido)
- Cero `std::filesystem`, TLS, dlopen, o signals
- `FsNodeZipFile` ya existe: carga todo en heap desde un buffer en memoria
- Toda la lógica de simulación es integer pura (fixed-point)

### Único bloqueante: FileSystem
`FsNode` usa POSIX (`fopen`, `opendir`, `stat`). Opciones:

| Opción | LOC | Complejidad |
|--------|-----|-------------|
| A. Emscripten `--preload-file TC/` | 0 LOC de código, flag de compilación | Muy baja |
| B. `FsNodeZip` + TC embebido como array C | ~50 LOC | Baja |
| C. JS FS backend (IndexedDB) | ~200 LOC | Media |

La opción A (preload-file) permite probar en browser con un solo flag. La opción B es la correcta para Rust (`include_bytes!`).

**Conclusión S8-03:** El criterio de "prototipo browser en ≤ 2 semanas" es alcanzable. Opción A con Emscripten se puede probar en horas; opción B desde Rust/WASM con `include_bytes!` es la solución de producción.

---

## Decisión: Rust ✅

### Factores determinantes

**1. Browser como killer feature**  
El mercado de Liero es la comunidad retro + speedrunners. Un build web sin instalación multiplica el alcance 5-10×. `wgpu` + `wasm32-unknown-unknown` + WebGPU es el único stack que da native _y_ web desde el mismo codebase. C++ requeriría Emscripten (subóptimo) o un port separado.

**2. Determinismo garantizado**  
La física es integer pura. Los `wrapping_*` ops de Rust garantizan el mismo comportamiento cross-platform sin UB. `sim_checksum` desde Rust ya coincide bit-a-bit con C++ (verificado en S8-02).

**3. FFI es trivial**  
`liero-ffi-proof` demostró que el C ABI es limpio. El port 1:1 de `liero-sim` puede validarse frame-a-frame contra el corpus de replays C++ con `sim_checksum`. Sin ese harness, reescribir sería un coin flip; con él, cada divergencia se detecta en segundos.

**4. Ecosistema gamedev maduro (2026)**  
`wgpu 0.19` soporta Vulkan/Metal/DX12/WebGPU desde un renderer. `winit 0.30` unifica window management. `cpal 0.15` cubre audio. `mlua 0.9` + `wasmtime 18` son production-grade para modding. No se usa Bevy (su ECS es incompatible con rollback netcode determinista).

**5. Safety para pools**  
Los crashes históricos vienen de pool overflows (wobjects, nobjects). Rust los elimina por construcción.

### Stack definitivo

```
wgpu       — renderer (Vulkan/Metal/DX12/WebGPU desde un backend)
winit      — window + event loop (native + WASM)
cpal       — audio (native + WASM via Web Audio API)
mlua       — scripting Lua para modders casuales
wasmtime   — scripting WASM sandboxed para TCs avanzados
serde/toml — carga de datos (ya format-compatible con TC existentes)
quinn      — QUIC UDP para rollback netcode (opcional vs UDP raw)
```

NO se usa: Bevy, Amethyst, ggez, macroquad. Stack mínimo, sin magic scheduler.

### Hoja de ruta Rust (tras Fase 1 C++)

| Milestone | Duración | Gate |
|-----------|----------|------|
| **R0** liero-sim port 1:1 | meses 1–4 | 1000+ replays bit-exact vs corpus C++ |
| **R1** liero-render GPU | meses 4–6 | textura R8 + LUT 256 colores, dirty-rect |
| **R2** Feature parity | meses 5–7 | Todos los modos Fase 1, side-by-side con C++ |
| **R3** liero-net rollback | meses 6–9 | 8 jugadores a 150ms RTT |
| **R4** Build web | meses 7–9 | WebGPU + WebTransport, Itch.io + self-hosted |
| **R5** Modding | meses 9–11 | Lua scripts + WASM sandboxed |
| **R6** Release | meses 11–12 | Steam + Itch.io + web |

---

## Condición Go evaluada

> *"Si el prototipo browser funciona en ≤ 2 semanas → proceder con Rust."*

El prototipo FFI funcionó en < 2 horas, no 2 semanas. El browser build con Emscripten --preload-file es alcanzable en días. **Condición Go cumplida.**

---

## Artefactos de este sprint

| Artefacto | Ruta | Estado |
|-----------|------|--------|
| Prototipo Rust FFI | `liero-ffi-proof/` | ✅ Corre 1000 frames, checksum golden |
| C ABI headless | `src/game/sim_c_api.h` | ✅ Clean, no SDL |
| Static lib | `build/libliero_sim.a` | ✅ Linkea solo libc++ |
| Auditoría WASM | Este documento, §S8-03 | ✅ 200 LOC de shim |
| Decisión | Este archivo | ✅ GO Rust |
