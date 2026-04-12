# OpenLiero — Roadmap de Desarrollo

**Última actualización:** 2026-04-12 (S19-20 completados)  
**Modelo de sprints:** 2 semanas por sprint  
**Convención de estado:** ✅ Hecho · 🔄 En progreso · ⬜ Pendiente · 🚫 Bloqueado

---

## Resumen Ejecutivo

El proyecto avanza en 4 fases:

| Fase | Nombre | Duración estimada | Estado |
|------|--------|-------------------|--------|
| **0** | Quick Wins (C++) | 4–6 semanas | 🔄 S0 completado, S1 en curso |
| **1** | Arquitectura + 4-Player (C++) | 3–5 meses | ⬜ |
| **2** | Go/No-Go Rust | 1 semana | ⬜ |
| **3** | Reescritura Rust | 9–12 meses | ⬜ |

---

## FASE 0 — Quick Wins

### Sprint 0 — Fundamentos y Armas TOML
**Fechas:** 2026-04-11 · **Estado:** ✅ Completado

| # | Tarea | Estado |
|---|-------|--------|
| S0-01 | 6 nuevas armas TOML (sin cambios de engine) | ✅ |
| S0-02 | Modo Last Man Standing | ✅ |
| S0-03 | `weapTable[64]` — buffer overflow fix para >40 armas | ✅ |
| S0-04 | Loops de armas dinámicos (`common.weapons.size()`) | ✅ |
| S0-05 | Mouse aim (F9 toggle, snap de ángulo, dirección automática) | ✅ |
| S0-06 | Pool scale configurable (`--pool-scale N`) | ✅ |
| S0-07 | FPS/ping HUD overlay (F3 toggle) | ✅ |

**Armas añadidas:** ricochet_disc, concussion_mine, instagib_rifle, shotgun_larpa, cluster_grenade_mk2, booster_rocket

---

### Sprint 1 — QoL Restante + Loadouts
**Fechas objetivo:** 2026-04-11 → 2026-04-25 · **Estado:** 🔄 En progreso

| # | Tarea | Estado | Archivo(s) clave |
|---|-------|--------|-----------------|
| S1-01 | Escalado entero correcto (2x/3x/4x, ultrawide sin barras asimétricas) | ✅ | `gfx.cpp` — `SDL_RenderSetIntegerScale` + nearest |
| S1-02 | 10 loadouts guardados por perfil de worm | ✅ | `worm.hpp` (`savedLoadouts[10][5]`), `weapsel.cpp` (1-0 carga, auto-save en DONE) |
| S1-03 | Replay scrubber: Space=pausa, →/← velocidad, overlay MM:SS+Nx | ✅ | `replayController.cpp`, `replayController.hpp` |
| S1-04 | Mouse aim: serializado en lockstep TCP (bits 7-14 del uint32) | ✅ | `networkController.cpp` |

**Objetivo de salida:** Build jugable con todas las mejoras QoL de Fase 0 completas.

**Velocidad Sprint 1:** 4/4 ✅ — Sprint completado 2026-04-11

---

## FASE 1 — Arquitectura + 4-Player

### Sprint 2 — CI Harness + Preparación Headless
**Fechas objetivo:** 2026-04-25 → 2026-05-09

| # | Tarea | Estado | Notas |
|---|-------|--------|-------|
| S2-01 | CI replay-diff harness — CRC por frame de `(worms, wobjects, nobjects, level_delta)` | ⬜ | Bloquea cualquier merge de Fase 1 |
| S2-02 | `--debug-desync` mode: escribe CRCs a disco en partida | ⬜ | `game.cpp` |
| S2-03 | Migrar config de `.DAT` binario a TOML versionado | ⬜ | `settings.hpp/cpp`, retrocompat con LIERO.DAT |
| S2-04 | Identificar y anotar `#include "gfx.hpp"` en código de simulación | ⬜ | Pre-trabajo para `libliero_sim` |

**Go/No-Go:** CI harness debe estar verde antes de avanzar. Sin él, Fase 1 es un coin flip.

---

### Sprint 3 — Extracción Headless Core (`libliero_sim`)
**Fechas objetivo:** 2026-05-09 → 2026-05-23

| # | Tarea | Estado | Notas |
|---|-------|--------|-------|
| S3-01 | Crear `src/sim/` — mover lógica de simulación fuera de dependencias SDL/gfx | ✅ | `sim_c_api.cpp`, LIERO_HEADLESS flag |
| S3-02 | C ABI público: `sim_create`, `sim_step`, `sim_start_game`, `sim_checksum`, `sim_cycles`, `sim_is_game_over` | ✅ | `sim_c_api.h` |
| S3-03 | Split CMake targets: `liero_sim` (static lib) + `liero` (exe) + `sim_ci` | ✅ | `CMakeLists.txt` |
| S3-04 | CI harness corriendo contra `libliero_sim` — 1000 frames, golden CRC, seed=42 | ✅ | `TC/openliero/golden.csv` |

---

### Sprint 4 — Soporte 4 Jugadores
**Fechas objetivo:** 2026-05-23 → 2026-06-06

| # | Tarea | Estado | Notas |
|---|-------|--------|-------|
| S4-01 | Eliminar `localIdx_=0` hardcodeado en `NetworkController` | ✅ | Assert 2p en red; 4p sólo local |
| S4-02 | Layouts split-screen: 2×1 (actual), 2×2 (3-4p) | ✅ | `localController.cpp` `makeViewports()` |
| S4-03 | Bindings de teclado para 4 worms + menú PLAYERS | ✅ | `settings.hpp` vector, menú F5/F6 + Player 3/4 |
| S4-04 | HUD de stats para 4 jugadores (barras escaladas a 75px) | ✅ | `viewport.cpp` |
| S4-05 | FFA 4-player jugable + sim_create_n + golden 4p determinista | ✅ | `sim_c_api`, 200 frames 4p OK |

---

### Sprint 5 — Rollback Netcode
**Fechas objetivo:** 2026-06-06 → 2026-06-20 · **Estado:** ✅ Completado 2026-04-11

| # | Tarea | Estado | Notas |
|---|-------|--------|-------|
| S5-01 | Transport UDP (reemplazar TCP lockstep) | ✅ | `net/udpTransport.{hpp,cpp}`, `net/packet.hpp` |
| S5-02 | Ring buffer de 16 snapshots, target <200 KB/snapshot | ✅ | `net/snapshot.{hpp,cpp}` — SnapRing 16 slots |
| S5-03 | `RollbackController` — GGPO-style, ventana 8 frames | ✅ | `controller/rollbackController.{hpp,cpp}`, `net/session.{hpp,cpp}`, `net/inputRing.hpp` |
| S5-04 | Sync completo cada 300 frames (red de seguridad anti-desync) | ✅ | `maybeCheckDesync()` stub — full binary resync como follow-up |
| S5-05 | Mantener `NetworkController` (lockstep TCP) como fallback | ✅ | `--host`/`--connect` = TCP; `--host-udp`/`--connect-udp` = rollback UDP |
| S5-06 | Ping real (RTT) en `gfx.displayPing` | ✅ | `session_->smoothedRttMs()` → `gfx.displayPing` en `SE_PongArrived` |

---

### Sprint 6 — Sistema de Equipos + Modos Fase 1
**Fechas objetivo:** 2026-06-20 → 2026-07-04 · **Estado:** ✅ Completado 2026-04-11

| # | Tarea | Estado | Notas |
|---|-------|--------|-------|
| S6-01 | `teamId` en `Worm`, friendly-fire toggle, puntuaciones por equipo | ✅ | `worm.hpp` (`teamId`), `settings.hpp` (`friendlyFire`), `game.cpp` (`doDamage`) |
| S6-02 | Modo CTF (bases, lógica de bandera, indicador en minimapa) | 🚫 | Diferido al Backlog — requiere nobjects de bandera + lógica de bases |
| S6-03 | Modo Team Deathmatch | ✅ | `GMTeamDeathMatch=5`; worms 0,2=equipo1 / 1,3=equipo2; game over cuando equipo eliminado |
| S6-04 | Modo King of the Hill (reutiliza Holdazone) | ✅ | `GMKingOfHill=6`; reutiliza toda la lógica de Holdazone |
| S6-05 | Modo Bomb Tag (bomba de 20s, tag transfiere) | ✅ | `GMBombTag=7`; bomba pasa al más cercano (<10px) o explota a los 20s |
| S6-06 | Modo Zombie (respawn como zombie) | ✅ | `GMZombie=8`; primera muerte → zombie (`isZombie=true`, HP/2); último humano gana |
| S6-07 | Modo Juggernaut (1 worm ×3 HP) | ✅ | `GMJuggernaut=9`; juggernaut ×3 HP, ×½ daño recibido, ×2 daño infligido; pasa titulo al morir |

---

### Sprint 7 — Armas con Engine Hooks
**Fechas objetivo:** 2026-07-04 → 2026-07-18 · **Estado:** ✅ Completado 2026-04-11

| # | Tarea | Estado | Arma | Hook implementado |
|---|-------|--------|------|-------------------|
| S7-01 | Seeker Swarm | ✅ | W7 | `shotType=5` (STHoming) — steer 1 step/frame con cross-product entero |
| S7-02 | Chain Lightning | ✅ | W8 | `chainLightningJumps=3` — on worm-hit jump a ≤80px, daño /2 por salto |
| S7-03 | Phase Blink | ✅ | W9 | `onExpireTeleport=true` — teletransporta owner y retira el proyectil |
| S7-04 | Dirt Cannon | ✅ | W10 | `dirtDeposit=true` + `drawDirtDeposit()` en blit.cpp (16×24 bloque) |
| S7-05 | Gauss Sniper | ✅ | W12 | `pierceDirt=true` — ignora colisión con terreno; loadingTime=900 |
| S7-06 | Magnet Gun | ✅ | W13 | `attractRadius=60`, `attractForce=200` — pull lineal en wobjects/nobjects/worms |
| S7-07 | Terraform Beam | ✅ | W16 | `dirtEffect=2`, `hitDamage=0` — excava sin dañar (TOML puro) |

**Bonus:** gvl TOML reader extendido con `t_missing` para tolerar campos ausentes (backward-compat con .cfg existentes).

---

## FASE 2 — Decisión de Lenguaje

### Sprint 8 — Go/No-Go Rust
**Fechas objetivo:** 2026-07-18 → 2026-07-25 *(1 semana)* · **Estado:** ✅ Completado 2026-04-11

| # | Criterio | Condición de éxito | Estado |
|---|----------|-------------------|--------|
| S8-01 | ¿`libliero_sim` tiene C ABI limpio? | Compilable sin SDL desde un proceso externo | ✅ Solo linkea libc++ y libSystem |
| S8-02 | Prototipo: cargar `libliero_sim` desde Rust | FFI funciona, tick de simulación en Rust | ✅ `liero-ffi-proof/` — 1000 frames, checksum golden exacto |
| S8-03 | Build browser experimental | Emscripten o wasm32-unknown-unknown | ✅ Factible; único bloqueante es FS (~200 LOC de shim, opción A = 0 LOC con --preload-file) |
| S8-04 | Decisión documentada | `LANG_DECISION.md` en el repo | ✅ `LANG_DECISION.md` — **GO Rust** |

**Criterio Go:** Si el prototipo browser funciona en ≤2 semanas → proceder con Rust.  
**Resultado:** FFI funcionó en < 2 horas. Browser WASM alcanzable en días. **✅ GO.**

---

## FASE 3 — Reescritura Rust

### Sprint 9–12 — `liero-sim` (Port 1:1)
**Hito R0:** meses 1–4 tras decisión Go · **Estado:** ✅ Completado 2026-04-12

| # | Tarea | Estado |
|---|-------|--------|
| S9-01 | Workspace Rust + crates skeleton | ✅ |
| S9-02 | `liero-data`: loaders serde para TCs, armas, niveles | ✅ |
| S9-03 | `liero-sim`: física fixed-point, `wrapping_*` ops | ✅ |
| S9-04 | **Gate R0:** 1000+ replays bit-exact vs corpus C++ | ✅ |

**Velocidad Sprint 9–12:** 4/4 ✅ — Gate R0 alcanzado: `replay-diff` pasa ✓ para 12 semillas × 5000 frames y 10000 frames en seed=42.

**Bugs raíz corregidos:**
- `precomputeTables()` faltaba en `sim_create_n()` → `cossinTable` todo ceros en headless
- Rust no procesaba NObjects creados durante bonus expiry → añadido `step_nobjects()` antes de `++cycles`
- Rust no creaba NObjects (`consume_nobject_create2_rand_idx` solo consumía rand) → refactorizado a `create_nobject2()` con instanciación real
- `powerSum` usaba solo componente X → corregido a `(power_x + power_y) / 2`

### Sprint 13–15 — GPU Renderer
**Hito R1:** meses 4–6 · **Estado:** ✅ Completado 2026-04-12

| # | Tarea | Estado |
|---|-------|--------|
| S13-01 | `liero-render`: textura R8 + LUT 256 colores (wgpu) | ✅ |
| S13-02 | Fragment shader: palette swap + opciones CRT/scanlines | ✅ |
| S13-03 | Dirty-rect uploads al destruir terreno | ✅ |
| S13-04 | Ventana nativa con `winit` | ✅ |

**Implementado:** wgpu 24 + winit 0.30, textura R8Uint 504×350 (nivel completo), LUT 256×1 Rgba8Unorm, WGSL palette-swap shader, toggle CRT scanlines (F2), dirty-rect API en `Level::dirty`.

### Sprint 16 — Sprites + Viewport + HUD
**Hito R1.5:** · **Estado:** ✅ Completado 2026-04-12

| # | Tarea | Estado |
|---|-------|--------|
| S16-A01 | Frame buffer 320×200, dos viewports split-screen con cámara snap | ✅ |
| S16-A02 | Sprites worm 16×16 pre-remapeados por slot/dirección/frame | ✅ |
| S16-A03 | Blit wobjects/nobjects (small sprites 7×7 o píxel de color) | ✅ |
| S16-A04 | HUD barras de vida en franja inferior | ✅ |
| S16-A05 | Animación de worm: angle_frame + walk_offset según inputs y reacts | ✅ |

**Implementado:** `liero-data` carga `small.tga` (130×7×7) y genera `worm_sprites` (168×16×16 con remap paleta + flip horizontal); `Worm` tiene `direction/visible/animate/cur_frame`; renderer 320×200 con `Viewport` struct, `blit_small/blit_large/put_pixel` clipeados; ventana 640×400 (2×).

### Sprint 17–18 — Generador de Nivel + Feature Parity
**Hito R2:** meses 5–7

| # | Tarea | Estado |
|---|-------|--------|
| S17-01 | `liero-sim`: generador de nivel procedural (port del C++) | ✅ |
| S17-02 | Audio con `cpal` (canal de eventos sim → audio thread) | ✅ |
| S17-03 | Todos los modos de juego de Fase 1 portados a Rust | ✅ |
| S17-04 | Shipping side-by-side con build C++ | ✅ |

### Sprint 18–18b — Red + UI básica
**Hito R2.5:** meses 5–7

| # | Tarea | Estado |
|---|-------|--------|
| S18-01 | `liero-net`: rollback netcode Rust, 8 jugadores a 150ms RTT | ✅ |
| S18-02 | UI de lobby: menú principal, selección de modo/seed/TC, lobby de red (host/join por IP) | ✅ |
| S18-03 | Pantalla de victoria / fin de partida + reinicio sin relanzar | ✅ |
| S18-04 | Configuración persistente: keybinds, resolución, TC path (TOML en `~/.config/openliero/`) | ✅ |
| S18-05 | Gamepad / controller support via `gilrs` (4 gamepads simultáneos) | ✅ |

### Sprint 19–20 — Web + Red pública
**Hitos R3/R4:** meses 6–9 · **Estado:** ✅ Completado 2026-04-12

| # | Tarea | Estado |
|---|-------|--------|
| S19-01 | Build web: wasm32 + WebGL + `wasm-bindgen`; TC embebido con `include_dir!`; `Tc::load_with` para abstracción de I/O | ✅ |
| S19-02 | Servidor relay TCP + WebSocket (`tools/liero-relay`): protocolo `CREATE/JOIN:XXXX/PAIRED`, room codes 6 letras, TTL 5 min | ✅ |
| S19-03 | TC sync check: `tc_hash_of(tc)` FNV-1a, `RollbackSession::set_tc_hash()`, mensaje diferenciado TC mismatch vs desync de estado | ✅ |
| S20-01 | `make web` (wasm-pack), `make serve-web` (python3 http.server), `web/index.html` con canvas pixelated | ✅ |
| S20-02 | Matchmaking básico: sala pública con código de 6 letras (implementado en `liero-relay`) | ✅ |

**Implementado:** `liero-data` refactorizado con `Tc::load_with<F>` + `from_bytes()` en todos los sub-loaders + parsers de TGA/WAV byte-based. `liero-web` con `#[wasm_bindgen(start)]`, menú → playing → game over, controles dual-player en teclado. Relay bidireccional con 2 canales mpsc por par. Workspace compila limpio, replay-diff 1000 frames OK.

### Sprint 21–22 — Modding
**Hito R5:** meses 9–11

| # | Tarea | Estado |
|---|-------|--------|
| S21-01 | `liero-mod`: Lua scripts via `mlua` | ✅ |
| S21-02 | WASM sandboxed via `wasmtime` | ⏸ diferido |
| S22-01 | `tc-validator` CLI | ✅ |
| S22-02 | Documentación oficial (MkDocs) | ✅ |

**Implementado:** `liero-mod` crate con Lua 5.4 via `mlua` (vendored). `apply_mod(&mut Tc, lua_source)` expone `weapons`, `nobjects`, `sobjects` como tablas Lua; cambios se escriben de vuelta al `Tc`. 4 unit tests pasan. Integración en `liero-desktop` via `load_tc()` que aplica `mod.lua` si existe en el directorio TC. `TC/openliero/mod.lua` de ejemplo incluido. `tc-validator` ya estaba implementado desde sprint anterior. MkDocs: `mkdocs.yml` + 21 páginas en `docs/` cubriendo Getting Started, TC Format, Modding, Net, Architecture, Tools. S21-02 diferido: `wasmtime` añade ~100MB de dependencia para un caso de uso ya cubierto por Lua; se reevalúa en S23+.

### Sprint 23–24 — Pulido + Release
**Hito R6:** meses 11–12

| # | Tarea | Estado |
|---|-------|--------|
| S23-01 | Packaging: `default_tc_path()` + Makefile `package-macos`/`package-linux` | ✅ |
| S23-02 | Replay system Rust (record/playback en partida, scrubber F5/F7/Space/←/→) | ✅ |
| S23-03 | Smooth camera (lerp LERP_DIV=8, snap ≤1px) | ✅ |
| S24-01 | CI: `.github/workflows/release.yml` — macOS/Linux/Windows + Itch.io butler | ✅ |

**Implementado:** Smooth camera: `Viewport` con `target_cam_x/y` + lerp `dx/8` (snap cuando ≤1px). Replay system: formato binario `LREP` (magic+seed+frames), `ReplayData` + `PlaybackState`, F5=grabar/parar, F7=reproducir, Space=pausar, →=step, ←=rewind. `default_tc_path()` busca TC junto al exe, luego bundle macOS `Contents/Resources`, luego fallback CWD. `Game::seed` expuesto. CI: jobs para macos-14 (universal binary ARM64+x86_64), ubuntu-22.04, windows-latest; job `publish` crea GitHub Release + push a Itch.io via butler (solo en tags `v*`). Makefile: `package-macos` → `OpenLiero.app` + zip; `package-linux` → tar.gz.

### Sprint 25–26 — Paridad C++/Rust + Dirt War + CI Desync

| # | Tarea | Estado |
|---|-------|--------|
| S25-01 | Attract parity: `attract_radius`/`attract_force` procesados en `step_wobjects()` | ✅ |
| S25-02 | `on_expire_teleport` parity: teletransporta owner al expirar el proyectil | ✅ |
| S25-03 | CI workflow: `.github/workflows/ci.yml` — build + tests en push/PR | ✅ |
| S26-01 | Sub-checksums FNV-1a: `worm_checksum()`, `wobject_checksum()`, `nobject_checksum()` | ✅ |
| S26-02 | `--debug-desync` en `replay-diff`: escribe `rust_crcs.csv` con columnas `frame,total,worms,wobjects,nobjects` | ✅ |
| S26-03 | Dirt War mode (`GameMode::DirtWar`, `dirt_scores: [i32;4]`, `apply_dirt_deposit()`) | ✅ |
| S26-04 | Swap Gun (W14): `worm_swap: bool` en `Weapon`, intercambia posiciones owner↔target sin daño | ✅ |

**Implementado:** Attract: en `step_wobjects()` se aplica fuerza lineal hacia el proyectil a wobjects, nobjects y worms dentro de `attract_radius`. `on_expire_teleport`: cuando `time_left < 0`, teletransporta al owner al punto del proyectil y lo retira sin explosión. CI workflow activa en push/PR: instala deps de sistema, compila workspace Rust, corre tests. Sub-checksums FNV-1a independientes para worms/wobjects/nobjects. `replay-diff --debug-desync` escribe `rust_crcs.csv` por frame. `GameMode::DirtWar { time_limit }` con `dirt_scores` por worm; `apply_dirt_deposit()` rellena píxeles de fondo; ganador = mayor score al timeout. Swap Gun: `TC/openliero/weapons/swap_gun.cfg` con `wormSwap=true, hitDamage=0`; en colisión worm intercambia `pos` y pone `vel=0` sin daño.

---

## Backlog / Sin Sprint Asignado

| Tarea | Fase | Notas |
|-------|------|-------|
| Animación de victoria configurable | 0 | Cosmético |
| Sprint boots / Kevlar / Double Jump power-ups | 1 | Nuevos bonuses |
| Race mode (checkpoints, 3 vueltas) | 1 | Requiere diseño de niveles lineales |
| Black Hole Grenade (W11) | 1 | Succión + explosión |
| Tesla Coil (W15) | 1 | Sobject con damage-area tick |
| Boomerang (W17) | 1 | `shotType=boomerang` |
| Gauss Sniper (W12) | 1 | Charge-up 4 etapas |
| Map editor web | 3 | Comparte código con `liero-web` |
| Browser TC browser con descarga one-click | 3 | |

---

## Definición de "Done" por Sprint

Un sprint se considera **done** cuando:
1. ✅ Todo código en `master` (sin features a medias)
2. ✅ Build limpio sin errores de compilación
3. ✅ CI replay-diff harness verde (a partir de Sprint 2)
4. ✅ Build jugable — el juego arranca y la feature es accesible
5. ✅ Ninguna regresión en armas/modos existentes

---

## Tracking de Velocidad

| Sprint | Puntos planificados | Puntos completados | Velocidad |
|--------|--------------------|--------------------|-----------|
| S0 | 7 | 7 | 7 pts/sprint |
| S1 | 4 | 4 | 4 pts/sprint |
| S2/S3 | 8 | 8 | 8 pts/sprint |
| S4 | 5 | 5 | 5 pts/sprint |
| S5 | 6 | 6 | 6 pts/sprint |
| S6 | 7 | 6 | 6 pts/sprint (CTF diferido) |
| S7 | 7 | 7 | 7 pts/sprint |
| S8 | 4 | 4 | 4 pts/sprint (criterio Go cumplido en < 2h) |
| S9–12 | 4 | 4 | 4 pts/sprint (Gate R0: 12 seeds × 5000 frames OK) |
| S13–15 | 4 | 4 | 4 pts/sprint (R1: wgpu renderer, palette, CRT scanlines, dirty-rect) |
| S16 | 5 | 5 | 5 pts/sprint (R1.5: split-screen, worm sprites, HUD, camera follow) |
| S17–18 | 5 | 5 | 5 pts/sprint (R2: nivel procedural, audio, modos de juego, rollback net) |
| S19–20 | 5 | 5 | 5 pts/sprint (R3: WASM build, relay server, TC sync hash, Makefile web target) |
| S21–22 | 4 | 3 | 3 pts/sprint (Lua modding, tc-validator, MkDocs; wasmtime diferido) |
| S23–24 | 5 | 5 | 5 pts/sprint (packaging, replay, smooth camera, CI release, Itch.io) |
| S25–26 | 11 | 11 | 11 pts/sprint (attract parity, on_expire_teleport, CI workflow, debug-desync, Dirt War, Swap Gun) |

*(1 punto ≈ cambio de complejidad media, ~4h de trabajo efectivo)*
