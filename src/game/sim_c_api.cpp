/* sim_c_api.cpp — Implementation of the C ABI for liero_sim.
 *
 * Compiled with LIERO_HEADLESS=1 so no SDL or rendering code is linked.
 */

#include "sim_c_api.h"

#include "game.hpp"
#include "worm.hpp"
#include "settings.hpp"
#include "common.hpp"
#include "filesystem.hpp"
#include "level.hpp"
#include "bonus.hpp"
#include "math.hpp"

#include <cstdio>
#include <stdexcept>

struct SimHandle
{
    gvl::shared_ptr<Common>   common;
    gvl::shared_ptr<Settings> settings;
    Game                      game;
    bool                      started;

    SimHandle(gvl::shared_ptr<Common> common_, gvl::shared_ptr<Settings> settings_, int numWorms)
    : common(common_)
    , settings(settings_)
    , game(common_, settings_,
           gvl::shared_ptr<SoundPlayer>(new NullSoundPlayer()))
    , started(false)
    {
        if (numWorms < 2) numWorms = 2;
        if (numWorms > 4) numWorms = 4;
        settings_->ensureWormCount(numWorms);

        // Add human-controlled worms (no AI in headless sim)
        static const int statsXs[4] = {0, 218, 0, 218};
        for (int i = 0; i < numWorms; ++i)
        {
            Worm* w = new Worm();
            w->settings = settings_->wormSettings[i];
            w->health   = w->settings->health;
            w->index    = i;
            w->statsX   = statsXs[i];
            game.addWorm(w);
        }
    }
};

SimHandle* sim_create_n(const char* tcPath, int numWorms)
{
    // Initialise lookup tables that main() normally sets up.
    precomputeTables();

    try
    {
        FsNode tcNode(tcPath);
        auto common = gvl::shared_ptr<Common>(new Common());
        common->load(std::move(tcNode));

        auto settings = gvl::shared_ptr<Settings>(new Settings());

        return new SimHandle(common, settings, numWorms);
    }
    catch (std::exception const& e)
    {
        std::fprintf(stderr, "sim_create_n: %s\n", e.what());
        return nullptr;
    }
}

SimHandle* sim_create(const char* tcPath)
{
    return sim_create_n(tcPath, 2);
}

void sim_destroy(SimHandle* sim)
{
    delete sim;
}

void sim_start_game(SimHandle* sim, uint32_t seed)
{
    if (!sim || sim->started) return;
    // Fix seed before level generation so runs are deterministic.
    sim->game.rand.seed(seed);
    // Generate level (done by gfx.cpp in the interactive build).
    sim->game.level.generateFromSettings(*sim->common, *sim->settings, sim->game.rand);
    // Initialize worm weapons (done by weapsel.cpp in the interactive build).
    for (std::size_t i = 0; i < sim->game.worms.size(); ++i)
    {
        Worm& w = *sim->game.worms[i];
        w.lives = sim->settings->lives;
        w.initWeapons(sim->game);
    }
    sim->game.startGame();
    sim->started = true;
}

void sim_step(SimHandle* sim, const sim_frame_input_t* inputs)
{
    if (!sim || !sim->started) return;

    int n = (inputs && inputs->num_worms > 0)
            ? inputs->num_worms
            : static_cast<int>(sim->game.worms.size());

    for (int i = 0; i < n && i < static_cast<int>(sim->game.worms.size()); ++i)
    {
        Worm& w = *sim->game.worms[i];
        if (inputs)
        {
            uint32_t raw = inputs->worms[i];
            w.controlStates.unpack(raw);
            if (raw & (1u << 7))
                w.mouseAimAngle = static_cast<int>((raw >> 8) & 0x7F);
            else
                w.mouseAimAngle = -1;
        }
    }

    sim->game.processFrame();
}

uint64_t sim_checksum(SimHandle* sim)
{
    if (!sim) return 0;
    return fullGameChecksum(sim->game);
}

int sim_cycles(SimHandle* sim)
{
    if (!sim) return 0;
    return sim->game.cycles;
}

int sim_is_game_over(SimHandle* sim)
{
    if (!sim) return 1;
    return sim->game.isGameOver() ? 1 : 0;
}

uint32_t sim_get_rand_x(SimHandle* sim)
{
    if (!sim) return 0;
    return sim->game.rand.x;
}

uint32_t sim_get_rand_c(SimHandle* sim)
{
    if (!sim) return 0;
    return sim->game.rand.c;
}

void sim_get_worm_pos(SimHandle* sim, int worm_idx,
                      int32_t* out_pos_x, int32_t* out_pos_y)
{
    if (!sim || !out_pos_x || !out_pos_y) return;
    if (worm_idx < 0 || worm_idx >= (int)sim->game.worms.size()) { *out_pos_x = *out_pos_y = 0; return; }
    *out_pos_x = sim->game.worms[worm_idx]->pos.x;
    *out_pos_y = sim->game.worms[worm_idx]->pos.y;
}

void sim_export_level(SimHandle* sim,
                      uint8_t*  out_pixels,
                      int32_t*  out_width,
                      int32_t*  out_height)
{
    if (!sim || !out_pixels || !out_width || !out_height) return;

    *out_width  = static_cast<int32_t>(sim->game.level.width);
    *out_height = static_cast<int32_t>(sim->game.level.height);

    for (int y = 0; y < *out_height; ++y)
    for (int x = 0; x < *out_width;  ++x)
        out_pixels[x + y * (*out_width)] = sim->game.level.pixel(x, y);
}

int32_t sim_get_bonus_count(SimHandle* sim)
{
    if (!sim) return 0;
    int32_t n = 0;
    auto br = sim->game.bonuses.all();
    Bonus* b;
    while ((b = br.next())) ++n;
    return n;
}

uint8_t sim_get_pixel(SimHandle* sim, int32_t x, int32_t y)
{
    if (!sim) return 255;
    int w = (int)sim->game.level.width;
    int h = (int)sim->game.level.height;
    if (x < 0 || y < 0 || x >= w || y >= h) return 255;
    return sim->game.level.pixel(x, y);
}

uint8_t sim_get_material_flags(SimHandle* sim, uint8_t pal_idx)
{
    if (!sim) return 0;
    return sim->common->materials[pal_idx].flags;
}

void sim_export_state(SimHandle* sim, sim_state_t* out)
{
    if (!sim || !out) return;
    std::memset(out, 0, sizeof(*out));

    out->rand_x   = sim->game.rand.x;
    out->rand_c   = sim->game.rand.c;
    out->cycles   = sim->game.cycles;
    out->num_worms = static_cast<int32_t>(sim->game.worms.size());

    for (int i = 0; i < out->num_worms && i < SIM_MAX_WORMS; ++i)
    {
        Worm const& w = *sim->game.worms[i];
        sim_worm_state_t& ws = out->worms[i];

        ws.pos_x          = w.pos.x;
        ws.pos_y          = w.pos.y;
        ws.vel_x          = w.vel.x;
        ws.vel_y          = w.vel.y;
        ws.aiming_angle   = w.aimingAngle;
        ws.health         = w.health;
        ws.lives          = w.lives;
        ws.kills          = w.kills;
        ws.timer          = w.timer;
        ws.current_weapon = w.currentWeapon;
        ws.killed_timer   = w.killedTimer;
        ws.visible        = w.visible ? 1 : 0;
    }
}
