/* net/inputRing.hpp — Per-frame input buffer for rollback netcode.
 *
 * Stores confirmed inputs keyed by frame number for both local and remote worms.
 * When a remote input for frame F arrives (possibly late), it is written here
 * so the rollback controller can replay F..current with corrected inputs.
 */
#pragma once

#include "packet.hpp"
#include <cstring>

static const int INPUT_RING_SIZE = 64; // must be power-of-2

struct WormInputEntry
{
    NetWormInput input;
    bool         confirmed; // false = predicted (repeat-last)
};

struct InputRing
{
    WormInputEntry local[INPUT_RING_SIZE];
    WormInputEntry remote[INPUT_RING_SIZE];

    InputRing()
    {
        std::memset(local,  0, sizeof(local));
        std::memset(remote, 0, sizeof(remote));
    }

    // Store a confirmed local input for the given frame.
    void setLocal(int frame, NetWormInput inp)
    {
        int slot = frame & (INPUT_RING_SIZE - 1);
        local[slot].input     = inp;
        local[slot].confirmed = true;
    }

    // Store a confirmed remote input for the given frame.
    void setRemote(int frame, NetWormInput inp)
    {
        int slot = frame & (INPUT_RING_SIZE - 1);
        remote[slot].input     = inp;
        remote[slot].confirmed = true;
    }

    // Get local input for frame (returns last confirmed if unset).
    NetWormInput getLocal(int frame) const
    {
        return local[frame & (INPUT_RING_SIZE - 1)].input;
    }

    // Get remote input for frame (may be a prediction).
    NetWormInput getRemote(int frame) const
    {
        return remote[frame & (INPUT_RING_SIZE - 1)].input;
    }

    bool isRemoteConfirmed(int frame) const
    {
        return remote[frame & (INPUT_RING_SIZE - 1)].confirmed;
    }

    // Predict remote input for frame F as last confirmed remote input (repeat-last).
    void predictRemote(int frame, int lastConfirmedFrame)
    {
        int slot = frame & (INPUT_RING_SIZE - 1);
        if (!remote[slot].confirmed)
        {
            NetWormInput last = (lastConfirmedFrame >= 0)
                ? getRemote(lastConfirmedFrame)
                : 0;
            remote[slot].input     = last;
            remote[slot].confirmed = false;
        }
    }

    // Clear all entries at and after frame (to allow re-prediction).
    void clearFrom(int fromFrame, int toFrame)
    {
        for (int f = fromFrame; f <= toFrame; ++f)
        {
            int slot = f & (INPUT_RING_SIZE - 1);
            remote[slot].confirmed = false;
            remote[slot].input     = 0;
        }
    }
};
