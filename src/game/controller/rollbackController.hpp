#ifndef LIERO_CONTROLLER_ROLLBACK_CONTROLLER_HPP
#define LIERO_CONTROLLER_ROLLBACK_CONTROLLER_HPP

#include "localController.hpp"
#include "../net/session.hpp"
#include "../net/snapshot.hpp"
#include "../net/inputRing.hpp"

#include <string>
#include <vector>
#include <memory>

// GGPO-style rollback controller.
// Wraps a LocalController game session with:
//   - UDP session (handshake, input FEC)
//   - Per-frame snapshots (16-slot ring)
//   - Input prediction (repeat-last) + rollback on misprediction
//   - Anti-desync checksum every 300 frames
struct RollbackController : LocalController
{
    // Host mode: binds port, waits for client.
    static RollbackController* createHost(
        int port,
        gvl::shared_ptr<Common> common,
        gvl::shared_ptr<Settings> settings);

    // Client mode: connects to host.
    static RollbackController* createClient(
        const std::string& host,
        int port,
        gvl::shared_ptr<Common> common,
        gvl::shared_ptr<Settings> settings);

    ~RollbackController();

    void focus()  override;
    bool process() override;

private:
    RollbackController(
        std::unique_ptr<Session> session,
        int localWormIdx,
        gvl::shared_ptr<Common> common,
        gvl::shared_ptr<Settings> settings);

    // Pack local worm's control state for the network.
    NetWormInput packLocalInput() const;

    // Apply an input to the remote worm for a specific frame (un-pack into worm).
    void applyInput(int wormIdx, NetWormInput inp);

    // Rollback to the earliest frame where remote prediction was wrong,
    // then re-simulate up to currentFrame.
    void doRollback(int wrongFrame, int currentFrame);

    // Send a periodic anti-desync checksum comparison.
    void maybeCheckDesync();

    std::unique_ptr<Session> session_;
    SnapRing                 snapRing_;
    InputRing                inputRing_;

    int localIdx_;   // 0 = host, 1 = client
    int remoteIdx_;  // 1 - localIdx_

    // Last confirmed remote frame
    int lastConfirmedRemoteFrame_;

    // Pending rollback: -1 = none
    int rollbackTo_;

    // Anti-desync: checksum of last sent comparison
    int  lastSyncCheckFrame_;
    bool connected_;

    // Stall: if remote is too far behind, stall for up to this many frames.
    static const int MAX_STALL_FRAMES = 8;
    int stallFramesLeft_;
};

#endif // LIERO_CONTROLLER_ROLLBACK_CONTROLLER_HPP
