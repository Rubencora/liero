/* net/snapshot.hpp — Game state snapshot for rollback netcode.
 *
 * captureSnapshot() copies all mutable simulation state into a GameSnap.
 * restoreSnapshot() writes it back in-place, preserving worm/object pointer
 * addresses so that cross-object pointers (WObject::firedBy, Ninjarope::anchor)
 * remain valid after restoration.
 */
#pragma once

#include "../game.hpp"
#include "../worm.hpp"
#include "../level.hpp"

#include <vector>
#include <cstdint>

// ---------------------------------------------------------------------------
// Per-worm mutable state (excludes shared_ptr<WormSettings>, shared_ptr<WormAI>
// which are stable across a match and must not be snapshotted).
// ---------------------------------------------------------------------------
struct WormSnap
{
    fixedvec        pos, vel;
    gvl::ivec2      logicRespawn;
    int             hotspotX, hotspotY;
    fixed           aimingAngle, aimingSpeed;
    bool            ableToJump, ableToDig, keyChangePressed, movable;
    bool            animate, visible, ready, flag, makeSightGreen;
    int             health, lives, kills;
    int             timer, killedTimer, currentFrame;
    int             flags;
    Ninjarope       ninjarope;  // Worm* anchor valid because worms are fixed-address
    int             currentWeapon, lastKilledByIdx, fireCone, leaveShellTimer;
    int             reacts[4];
    WormWeapon      weapons[5];
    int             direction;
    Worm::ControlState controlStates, prevControlStates, cleanControlStates;
    int             steerableSumX, steerableSumY, steerableCount;
    int             mouseAimAngle;
};

// ---------------------------------------------------------------------------
// Full game snapshot (one ring slot).
// ---------------------------------------------------------------------------
struct GameSnap
{
    bool valid;   // whether this slot contains a captured state
    int  frame;   // value of game.cycles at capture time

    // PRNG
    uint32_t randX, randC;

    // Core game fields
    int  cycles;
    int  screenFlash;
    bool gotChanged;
    int  lastKilledIdx;
    bool paused;

    // Zone mode
    Holdazone holdazone;

    // Worms (up to 4)
    int      numWorms;
    WormSnap worms[4];

    // Object pools — full copy via vector<T> copy assignment
    Game::WObjectList wobjects;
    Game::SObjectList sobjects;
    Game::NObjectList nobjects;
    Game::BonusList   bonuses;
    Game::BObjectList bobjects;

    // Level terrain (pixels + materials)
    std::vector<uint8_t>   terrain;
    std::vector<Material>  terrainMats;

    GameSnap() : valid(false), frame(-1), numWorms(0) {}
};

// ---------------------------------------------------------------------------
// Ring buffer of 16 snapshots.
// ---------------------------------------------------------------------------
static const int SNAP_RING_SIZE = 16;

struct SnapRing
{
    GameSnap snaps[SNAP_RING_SIZE];
    int      head = 0;  // next slot to write

    // Capture current game state into the next ring slot.
    void capture(Game& game);

    // Find and restore the snapshot nearest to targetFrame (≤ targetFrame).
    // Returns the frame of the restored snapshot, or -1 if not found.
    int restoreTo(Game& game, int targetFrame);

    // Returns the most-recently captured snapshot frame, or -1.
    int latestFrame() const;
};

// ---------------------------------------------------------------------------
// Low-level helpers (also used by SnapRing).
// ---------------------------------------------------------------------------
void captureSnapshot(Game& game, GameSnap& snap);
void restoreSnapshot(GameSnap& snap, Game& game);
