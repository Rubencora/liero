/* net/snapshot.cpp — Snapshot capture and restore for rollback netcode. */
#include "snapshot.hpp"

#include <cstring>
#include <algorithm>

// ---------------------------------------------------------------------------
// Helper: copy mutable worm state in/out of a WormSnap.
// ---------------------------------------------------------------------------
static void captureWorm(const Worm& src, WormSnap& dst)
{
    dst.pos              = src.pos;
    dst.vel              = src.vel;
    dst.logicRespawn     = src.logicRespawn;
    dst.hotspotX         = src.hotspotX;
    dst.hotspotY         = src.hotspotY;
    dst.aimingAngle      = src.aimingAngle;
    dst.aimingSpeed      = src.aimingSpeed;
    dst.ableToJump       = src.ableToJump;
    dst.ableToDig        = src.ableToDig;
    dst.keyChangePressed = src.keyChangePressed;
    dst.movable          = src.movable;
    dst.animate          = src.animate;
    dst.visible          = src.visible;
    dst.ready            = src.ready;
    dst.flag             = src.flag;
    dst.makeSightGreen   = src.makeSightGreen;
    dst.health           = src.health;
    dst.lives            = src.lives;
    dst.kills            = src.kills;
    dst.timer            = src.timer;
    dst.killedTimer      = src.killedTimer;
    dst.currentFrame     = src.currentFrame;
    dst.flags            = src.flags;
    dst.ninjarope        = src.ninjarope; // Worm* anchor — valid (fixed address)
    dst.currentWeapon    = src.currentWeapon;
    dst.lastKilledByIdx  = src.lastKilledByIdx;
    dst.fireCone         = src.fireCone;
    dst.leaveShellTimer  = src.leaveShellTimer;
    std::copy(src.reacts, src.reacts + 4, dst.reacts);
    std::copy(src.weapons, src.weapons + 5, dst.weapons);
    // WormWeapon::type points to Common (stable), WormWeapon::firedBy not stored in snap
    dst.direction           = src.direction;
    dst.controlStates       = src.controlStates;
    dst.prevControlStates   = src.prevControlStates;
    dst.cleanControlStates  = src.cleanControlStates;
    dst.steerableSumX       = src.steerableSumX;
    dst.steerableSumY       = src.steerableSumY;
    dst.steerableCount      = src.steerableCount;
    dst.mouseAimAngle       = src.mouseAimAngle;
}

static void restoreWorm(const WormSnap& src, Worm& dst)
{
    dst.pos              = src.pos;
    dst.vel              = src.vel;
    dst.logicRespawn     = src.logicRespawn;
    dst.hotspotX         = src.hotspotX;
    dst.hotspotY         = src.hotspotY;
    dst.aimingAngle      = src.aimingAngle;
    dst.aimingSpeed      = src.aimingSpeed;
    dst.ableToJump       = src.ableToJump;
    dst.ableToDig        = src.ableToDig;
    dst.keyChangePressed = src.keyChangePressed;
    dst.movable          = src.movable;
    dst.animate          = src.animate;
    dst.visible          = src.visible;
    dst.ready            = src.ready;
    dst.flag             = src.flag;
    dst.makeSightGreen   = src.makeSightGreen;
    dst.health           = src.health;
    dst.lives            = src.lives;
    dst.kills            = src.kills;
    dst.timer            = src.timer;
    dst.killedTimer      = src.killedTimer;
    dst.currentFrame     = src.currentFrame;
    dst.flags            = src.flags;
    dst.ninjarope        = src.ninjarope;
    dst.currentWeapon    = src.currentWeapon;
    dst.lastKilledByIdx  = src.lastKilledByIdx;
    dst.fireCone         = src.fireCone;
    dst.leaveShellTimer  = src.leaveShellTimer;
    std::copy(src.reacts, src.reacts + 4, dst.reacts);
    std::copy(src.weapons, src.weapons + 5, dst.weapons);
    dst.direction           = src.direction;
    dst.controlStates       = src.controlStates;
    dst.prevControlStates   = src.prevControlStates;
    dst.cleanControlStates  = src.cleanControlStates;
    dst.steerableSumX       = src.steerableSumX;
    dst.steerableSumY       = src.steerableSumY;
    dst.steerableCount      = src.steerableCount;
    dst.mouseAimAngle       = src.mouseAimAngle;
    // Note: dst.settings and dst.ai are intentionally NOT overwritten —
    // they are stable across the match.
}

// ---------------------------------------------------------------------------
// captureSnapshot / restoreSnapshot
// ---------------------------------------------------------------------------
void captureSnapshot(Game& game, GameSnap& snap)
{
    snap.valid        = true;
    snap.frame        = game.cycles;
    snap.randX        = game.rand.x;
    snap.randC        = game.rand.c;
    snap.cycles       = game.cycles;
    snap.screenFlash  = game.screenFlash;
    snap.gotChanged   = game.gotChanged;
    snap.lastKilledIdx= game.lastKilledIdx;
    snap.paused       = game.paused;
    snap.holdazone    = game.holdazone;

    snap.numWorms = static_cast<int>(game.worms.size());
    for (int i = 0; i < snap.numWorms && i < 4; ++i)
        captureWorm(*game.worms[i], snap.worms[i]);

    // Object pools — vector copy (deep copy of all slots, used and free).
    snap.wobjects = game.wobjects;
    snap.sobjects = game.sobjects;
    snap.nobjects = game.nobjects;
    snap.bonuses  = game.bonuses;
    snap.bobjects = game.bobjects;

    // Level terrain
    snap.terrain     = game.level.data;
    snap.terrainMats = game.level.materials;
}

void restoreSnapshot(GameSnap& snap, Game& game)
{
    if (!snap.valid) return;

    game.rand.x        = snap.randX;
    game.rand.c        = snap.randC;
    game.cycles        = snap.cycles;
    game.screenFlash   = snap.screenFlash;
    game.gotChanged    = snap.gotChanged;
    game.lastKilledIdx = snap.lastKilledIdx;
    game.paused        = snap.paused;
    game.holdazone     = snap.holdazone;

    int n = std::min(snap.numWorms, static_cast<int>(game.worms.size()));
    for (int i = 0; i < n; ++i)
        restoreWorm(snap.worms[i], *game.worms[i]);

    // Restore object pools in-place (preserves pool addresses).
    game.wobjects = snap.wobjects;
    game.sobjects = snap.sobjects;
    game.nobjects = snap.nobjects;
    game.bonuses  = snap.bonuses;
    game.bobjects = snap.bobjects;

    // Restore terrain
    game.level.data      = snap.terrain;
    game.level.materials = snap.terrainMats;
}

// ---------------------------------------------------------------------------
// SnapRing
// ---------------------------------------------------------------------------
void SnapRing::capture(Game& game)
{
    captureSnapshot(game, snaps[head]);
    head = (head + 1) % SNAP_RING_SIZE;
}

int SnapRing::restoreTo(Game& game, int targetFrame)
{
    // Find the most-recent snapshot with frame <= targetFrame.
    int bestSlot = -1;
    int bestFrame = -1;
    for (int i = 0; i < SNAP_RING_SIZE; ++i)
    {
        if (snaps[i].valid && snaps[i].frame <= targetFrame && snaps[i].frame > bestFrame)
        {
            bestFrame = snaps[i].frame;
            bestSlot  = i;
        }
    }
    if (bestSlot < 0) return -1;
    restoreSnapshot(snaps[bestSlot], game);
    return bestFrame;
}

int SnapRing::latestFrame() const
{
    int latest = -1;
    for (int i = 0; i < SNAP_RING_SIZE; ++i)
        if (snaps[i].valid && snaps[i].frame > latest)
            latest = snaps[i].frame;
    return latest;
}
