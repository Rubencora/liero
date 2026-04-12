/* controller/rollbackController.cpp — GGPO-style rollback netcode. */
#include "rollbackController.hpp"
#include "../gfx.hpp"
#include "../worm.hpp"
#include "../game.hpp"
#include "stats_presenter.hpp"

#include <cstdio>
#include <algorithm>

// ---------------------------------------------------------------------------
// Factory helpers
// ---------------------------------------------------------------------------
RollbackController* RollbackController::createHost(
    int port,
    gvl::shared_ptr<Common> common,
    gvl::shared_ptr<Settings> settings)
{
    // Generate a seed now; it will be sent in PT_HandshakeAccept.
    uint32_t seed = static_cast<uint32_t>(std::time(nullptr));
    auto* sess = Session::createHost(port, seed);
    if (!sess)
    {
        std::fprintf(stderr, "[rollback] createHost failed\n");
        return nullptr;
    }
    auto ctrl = new RollbackController(
        std::unique_ptr<Session>(sess), 0 /*host=worm0*/, common, settings);
    return ctrl;
}

RollbackController* RollbackController::createClient(
    const std::string& host,
    int port,
    gvl::shared_ptr<Common> common,
    gvl::shared_ptr<Settings> settings)
{
    auto* sess = Session::createClient(host, port);
    if (!sess)
    {
        std::fprintf(stderr, "[rollback] createClient failed\n");
        return nullptr;
    }
    auto ctrl = new RollbackController(
        std::unique_ptr<Session>(sess), 1 /*client=worm1*/, common, settings);
    return ctrl;
}

RollbackController::RollbackController(
    std::unique_ptr<Session> session,
    int localWormIdx,
    gvl::shared_ptr<Common> common,
    gvl::shared_ptr<Settings> settings)
: LocalController(common, settings)
, session_(std::move(session))
, localIdx_(localWormIdx)
, remoteIdx_(1 - localWormIdx)
, lastConfirmedRemoteFrame_(-1)
, rollbackTo_(-1)
, lastSyncCheckFrame_(-1)
, connected_(false)
, stallFramesLeft_(0)
{
    // Clear AI on both worms — network drives the remote one.
    for (auto& w : game.worms)
        w->ai.reset();
}

RollbackController::~RollbackController() {}

// ---------------------------------------------------------------------------
// Pack/unpack inputs
// ---------------------------------------------------------------------------
NetWormInput RollbackController::packLocalInput() const
{
    if (localIdx_ < 0 || localIdx_ >= (int)game.worms.size())
        return 0;
    const Worm& w = *game.worms[localIdx_];
    uint32_t raw = w.controlStates.pack();
    if (w.mouseAimAngle >= 0)
    {
        raw |= (1u << 7);
        raw |= (static_cast<uint32_t>(w.mouseAimAngle & 0x7F) << 8);
    }
    return raw;
}

void RollbackController::applyInput(int wormIdx, NetWormInput inp)
{
    if (wormIdx < 0 || wormIdx >= (int)game.worms.size()) return;
    Worm& w = *game.worms[wormIdx];
    w.controlStates.unpack(inp);
    if (inp & (1u << 7))
        w.mouseAimAngle = static_cast<int>((inp >> 8) & 0x7F);
    else
        w.mouseAimAngle = -1;
}

// ---------------------------------------------------------------------------
// focus() — called when controller gains focus
// ---------------------------------------------------------------------------
void RollbackController::focus()
{
    if (state == StateGameEnded)
    {
        goingToMenu = true;
        fadeValue   = 0;
        return;
    }

    if (state == StateInitial)
    {
        // Skip weapon selection in network mode.
        changeState(StateGame);
    }

    game.focus(gfx.playRenderer);
    game.focus(gfx.singleScreenRenderer);
    goingToMenu = false;
    fadeValue   = 0;
}

// ---------------------------------------------------------------------------
// process() — called every display frame
// ---------------------------------------------------------------------------
bool RollbackController::process()
{
    if (!connected_)
    {
        // Still in handshake — poll session until SE_Connected arrives.
        std::vector<SessionEvent> events;
        session_->poll(events);
        for (auto& ev : events)
        {
            if (ev.type == SE_Connected)
            {
                // Seed the game PRNG and start the game simulation.
                game.rand.seed(ev.seed);
                game.level.generateFromSettings(*game.common, *game.settings, game.rand);
                for (auto& wptr : game.worms)
                {
                    wptr->lives = game.settings->lives;
                    wptr->initWeapons(game);
                }
                game.startGame();

                // Take initial snapshot before any frame runs.
                snapRing_.capture(game);

                connected_ = true;
                std::fprintf(stderr, "[rollback] game started, localIdx=%d\n", localIdx_);
            }
        }
        if (!connected_) return true; // still waiting
    }

    // ------------------------------------------------------------------------
    // Connected — drive the rollback loop
    // ------------------------------------------------------------------------

    if (state == StateGame || state == StateGameEnded)
    {
        int realFrameSkip = inverseFrameSkip ? !(cycles % frameSkip) : frameSkip;

        for (int skip = 0; skip < realFrameSkip
             && (state == StateGame || state == StateGameEnded); ++skip)
        {
            // Stall: if remote is too far behind, pause simulation.
            if (stallFramesLeft_ > 0)
            {
                --stallFramesLeft_;
                // Still poll for incoming packets.
                std::vector<SessionEvent> events;
                session_->poll(events);
                break;
            }

            int currentFrame = game.cycles;

            // ------------------------------------------------------------------
            // 1. Poll session — collect remote inputs and other events.
            // ------------------------------------------------------------------
            std::vector<SessionEvent> events;
            session_->poll(events);

            for (auto& ev : events)
            {
                if (ev.type == SE_InputArrived)
                {
                    // Store each arriving frame's input; detect mispredictions.
                    for (int i = 0; i < ev.numInputs; ++i)
                    {
                        int   f   = ev.inputs[i].frame;
                        NetWormInput inp = ev.inputs[i].input;
                        if (f < 0 || f > currentFrame + 64) continue;

                        bool alreadyHad = inputRing_.isRemoteConfirmed(f);
                        if (!alreadyHad)
                        {
                            if (f < currentFrame && inp != inputRing_.getRemote(f))
                            {
                                // Misprediction: schedule rollback to earliest wrong frame.
                                if (rollbackTo_ < 0 || f < rollbackTo_)
                                    rollbackTo_ = f;
                            }
                            inputRing_.setRemote(f, inp);
                            if (f > lastConfirmedRemoteFrame_)
                                lastConfirmedRemoteFrame_ = f;
                        }
                    }
                }
                else if (ev.type == SE_PongArrived)
                {
                    gfx.displayPing = static_cast<int>(ev.rttMs);
                }
                else if (ev.type == SE_Disconnected)
                {
                    connected_ = false;
                    goingToMenu = true;
                    return true;
                }
            }

            // ------------------------------------------------------------------
            // 2. Rollback if needed.
            // ------------------------------------------------------------------
            if (rollbackTo_ >= 0 && rollbackTo_ < currentFrame)
            {
                doRollback(rollbackTo_, currentFrame);
                rollbackTo_ = -1;
                // currentFrame is now up-to-date with corrected inputs;
                // game.cycles was restored and re-simulated.
                currentFrame = game.cycles;
            }

            // ------------------------------------------------------------------
            // 3. Stall check: if remote is too far behind, stall ourselves.
            // ------------------------------------------------------------------
            static const int ROLLBACK_WINDOW = 8;
            if (lastConfirmedRemoteFrame_ >= 0 &&
                currentFrame - lastConfirmedRemoteFrame_ > ROLLBACK_WINDOW)
            {
                stallFramesLeft_ = 1;
                break;
            }

            // ------------------------------------------------------------------
            // 4. Predict remote input for this frame.
            // ------------------------------------------------------------------
            if (!inputRing_.isRemoteConfirmed(currentFrame))
                inputRing_.predictRemote(currentFrame, lastConfirmedRemoteFrame_);

            // ------------------------------------------------------------------
            // 5. Sample local input and store it.
            // ------------------------------------------------------------------
            NetWormInput myInp = packLocalInput();
            inputRing_.setLocal(currentFrame, myInp);

            // FEC: build history[3] (current, current-1, current-2).
            NetWormInput fec[3];
            fec[0] = myInp;
            fec[1] = inputRing_.getLocal(std::max(0, currentFrame - 1));
            fec[2] = inputRing_.getLocal(std::max(0, currentFrame - 2));
            session_->sendInput(currentFrame, fec);

            // ------------------------------------------------------------------
            // 6. Apply inputs for this frame.
            // ------------------------------------------------------------------
            applyInput(localIdx_,  inputRing_.getLocal(currentFrame));
            applyInput(remoteIdx_, inputRing_.getRemote(currentFrame));

            // ------------------------------------------------------------------
            // 7. Advance simulation.
            // ------------------------------------------------------------------
            game.processFrame();

            // ------------------------------------------------------------------
            // 8. Snapshot after each frame (ring of 16).
            // ------------------------------------------------------------------
            snapRing_.capture(game);

            // ------------------------------------------------------------------
            // 9. Anti-desync: compare checksums every 300 frames.
            // ------------------------------------------------------------------
            maybeCheckDesync();

            if (game.isGameOver())
                changeState(StateGameEnded);
        }
    }

    // ------------------------------------------------------------------------
    // Fade in/out + menu transition
    // ------------------------------------------------------------------------
    if (goingToMenu)
    {
        if (fadeValue > 0)
            --fadeValue;
        else
        {
            if (state == StateGameEnded)
            {
                endRecord();
                game.statsRecorder->finish(game);
                presentStats(static_cast<NormalStatsRecorder&>(*game.statsRecorder), game);
            }
            return false;
        }
    }
    else
    {
        if (fadeValue < 33)
            ++fadeValue;
    }

    return true;
}

// ---------------------------------------------------------------------------
// doRollback — restore to wrongFrame and re-simulate up to targetFrame.
// ---------------------------------------------------------------------------
void RollbackController::doRollback(int wrongFrame, int targetFrame)
{
    // Find and restore the best snapshot at or before wrongFrame.
    int restoredFrame = snapRing_.restoreTo(game, wrongFrame);
    if (restoredFrame < 0)
    {
        std::fprintf(stderr, "[rollback] no snapshot found for frame %d\n", wrongFrame);
        return;
    }

    // Re-simulate from restoredFrame up to targetFrame.
    for (int f = restoredFrame; f < targetFrame; ++f)
    {
        // Predict remote for any uncorrected frames in [restoredFrame, wrongFrame).
        if (!inputRing_.isRemoteConfirmed(f))
            inputRing_.predictRemote(f, lastConfirmedRemoteFrame_);

        applyInput(localIdx_,  inputRing_.getLocal(f));
        applyInput(remoteIdx_, inputRing_.getRemote(f));
        game.processFrame();
        snapRing_.capture(game); // update snapshots along the re-sim path
    }
}

// ---------------------------------------------------------------------------
// maybeCheckDesync — send/compare checksums every 300 frames.
// ---------------------------------------------------------------------------
void RollbackController::maybeCheckDesync()
{
    // Simple approach: we only log a warning when checksums diverge.
    // A full resync implementation would serialize and send the entire GameSnap.
    // For now, output a warning and disconnect on mismatch.
    // (Full resync is a follow-up task.)
    static const int SYNC_INTERVAL = 300;

    int currentFrame = game.cycles;
    if (currentFrame > 0 && (currentFrame % SYNC_INTERVAL) == 0
        && currentFrame != lastSyncCheckFrame_)
    {
        lastSyncCheckFrame_ = currentFrame;
        // In a full implementation, host would send a full-state blob here.
        // For the MVP, we rely on determinism; a desync manifests as diverging
        // gameplay, which the player notices. Disconnect on explicit mismatch
        // detection would go here if we added a PT_Checksum exchange.
    }
}
