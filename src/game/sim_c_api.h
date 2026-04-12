/* sim_c_api.h — Public C ABI for the liero_sim headless simulation library.
 *
 * Use this interface to drive a deterministic Liero game simulation
 * without any SDL/rendering dependencies.
 */
#pragma once

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle returned by sim_create(). */
typedef struct SimHandle SimHandle;

/* Per-worm input for one frame. Bit layout mirrors the lockstep TCP protocol:
 *   bits 0-6  : WormControlStates packed (Up/Down/Left/Right/Fire/Jump/Change/Dig)
 *   bit  7    : mouse aim active
 *   bits 8-14 : mouse aim angle (0-127, liero range 12..116)
 */
typedef uint32_t sim_worm_input_t;

/* Input bundle for one frame — up to 4 worms. */
typedef struct {
    sim_worm_input_t worms[4];
    int              num_worms;
} sim_frame_input_t;

/* Create a new simulation instance with the default 2 worms.
 * tcPath: filesystem path to the TC directory (e.g. "TC/openliero").
 * Returns NULL on failure (check stderr for diagnostics).
 */
SimHandle* sim_create(const char* tcPath);

/* Create a simulation instance with numWorms worms (2..4). */
SimHandle* sim_create_n(const char* tcPath, int numWorms);

/* Destroy a simulation instance and free all resources. */
void sim_destroy(SimHandle* sim);

/* Advance the simulation by one frame applying the given inputs. */
void sim_step(SimHandle* sim, const sim_frame_input_t* inputs);

/* Return a deterministic 64-bit checksum of the full simulation state.
 * Covers worms, wobjects, nobjects, bonuses, level, rand, and cycle count.
 */
uint64_t sim_checksum(SimHandle* sim);

/* Return the current cycle count (frames simulated since sim_start_game). */
int sim_cycles(SimHandle* sim);

/* Start a new game (place worms, reset state). Must be called after sim_create
 * and before sim_step.
 * seed: PRNG seed for deterministic replay (use the same seed to reproduce a run). */
void sim_start_game(SimHandle* sim, uint32_t seed);

/* Returns 1 if the game is over (all worms dead / time limit reached). */
int sim_is_game_over(SimHandle* sim);

/* -------------------------------------------------------------------------
 * State export/import — used by the Rust replay-diff harness to sync the
 * initial simulation state so both C++ and Rust sims start from identical
 * values even though Rust does not implement the level generator.
 * ----------------------------------------------------------------------- */

#define SIM_MAX_WORMS 4

typedef struct {
    int32_t  pos_x, pos_y;     /* Q16.16 fixed-point world position */
    int32_t  vel_x, vel_y;     /* Q16.16 fixed-point velocity        */
    int32_t  aiming_angle;     /* 0-127 into cossin table            */
    int32_t  health;
    int32_t  lives;
    int32_t  kills;
    int32_t  timer;
    int32_t  current_weapon;
    int32_t  killed_timer;     /* -1 = respawning, 0 = dead, >0 = countdown */
    int32_t  visible;          /* 1 = alive and visible, 0 = dead    */
} sim_worm_state_t;

typedef struct {
    uint32_t          rand_x;              /* MWC PRNG x register */
    uint32_t          rand_c;              /* MWC PRNG carry      */
    int32_t           cycles;             /* frame counter        */
    int32_t           num_worms;
    sim_worm_state_t  worms[SIM_MAX_WORMS];
} sim_state_t;

/* Export the full simulation state into *out.
 * Call immediately after sim_start_game() to capture the initial state.
 * The Rust harness imports this to bypass implementing the level generator. */
void sim_export_state(SimHandle* sim, sim_state_t* out);

/* Return the current PRNG x register (for determinism debugging). */
uint32_t sim_get_rand_x(SimHandle* sim);

/* Return the current PRNG carry (c) register. */
uint32_t sim_get_rand_c(SimHandle* sim);

/* Return worm position after last step (fixed-point). */
void sim_get_worm_pos(SimHandle* sim, int worm_idx,
                      int32_t* out_pos_x, int32_t* out_pos_y);

/* Export the level pixel data (palette indices, row-major, x + y*width).
 * out_pixels must point to a buffer of at least (*out_width) * (*out_height) bytes.
 * Call after sim_start_game() to get the generated terrain. */
void sim_export_level(SimHandle* sim,
                      uint8_t*  out_pixels,
                      int32_t*  out_width,
                      int32_t*  out_height);

/* Return the number of active bonuses. */
int32_t sim_get_bonus_count(SimHandle* sim);

/* Return the palette index of the pixel at (x, y). Returns 255 if OOB. */
uint8_t sim_get_pixel(SimHandle* sim, int32_t x, int32_t y);

/* Return the material flags byte for palette index pal_idx (from common.materials). */
uint8_t sim_get_material_flags(SimHandle* sim, uint8_t pal_idx);

#ifdef __cplusplus
} /* extern "C" */
#endif
