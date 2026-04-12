/* net/session.hpp — UDP session for rollback netcode.
 *
 * Manages the UDP connection lifecycle:
 *   1. 3-way handshake (Hello → Accept → first Input)
 *   2. Reliable-enough delivery via per-packet ack-bitmask
 *   3. PING/PONG RTT measurement
 *   4. Full-state sync reassembly (PT_FullSyncHdr + PT_FullSyncChunk)
 *
 * Session is polled every frame via poll(), which delivers zero or more
 * SessionEvent structs to the caller.
 */
#pragma once

#include "udpTransport.hpp"
#include "packet.hpp"
#include <vector>
#include <string>
#include <cstdint>

extern "C" {
#include <tl/socket.h>
}

// ---------------------------------------------------------------------------
// Events delivered by Session::poll()
// ---------------------------------------------------------------------------
enum SessionEventType
{
    SE_Connected,      // handshake complete; seed is valid
    SE_InputArrived,   // remote input packet decoded
    SE_PongArrived,    // RTT sample ready
    SE_FullSyncReady,  // full sync blob fully reassembled
    SE_Disconnected,
};

struct SessionEvent
{
    SessionEventType type;

    // SE_Connected
    uint32_t seed;
    int      localWormIdx;  // 0 = host, 1 = client

    // SE_InputArrived: one frame's worth of remote input (FEC: up to 3)
    struct InputEntry {
        int          frame;
        NetWormInput input;
    };
    InputEntry inputs[3];
    int        numInputs;

    // SE_PongArrived
    uint32_t rttMs;

    // SE_FullSyncReady
    std::vector<uint8_t> syncBlob;
    int                  syncFrame;
};

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------
struct Session
{
    // Create in host mode: binds port, waits for client UDP datagram.
    static Session* createHost(int port, uint32_t seed);

    // Create in client mode: sends Hello to host.
    static Session* createClient(const std::string& host, int port);

    ~Session();

    // Drive I/O: call once per frame. Appends any decoded events.
    void poll(std::vector<SessionEvent>& events);

    // Send local input for a frame (with FEC: 3 frames history).
    void sendInput(int frame, const NetWormInput history[3]);

    // Send a PING probe.
    void sendPing();

    // Send a full-state sync blob (host only). Fragments automatically.
    void sendFullSync(int frame, const std::vector<uint8_t>& blob);

    // Smoothed RTT in milliseconds (-1 if unknown).
    int smoothedRttMs() const { return smoothedRtt_; }

    bool isHost() const { return isHost_; }
    bool isConnected() const { return state_ == StateConnected; }

private:
    Session(bool isHost, UdpTransport* transport, uint32_t seed, int localIdx);

    enum State { StateHandshake, StateConnected, StateDisconnected };

    void processPacket(const uint8_t* buf, int len, const tl_internet_addr& src,
                       std::vector<SessionEvent>& events);
    void handleInput(const PktInput* pkt, std::vector<SessionEvent>& events);
    void handlePing(const PktPing* pkt);
    void handlePong(const PktPong* pkt, std::vector<SessionEvent>& events);
    void handleFullSyncHdr(const PktFullSyncHdr* pkt);
    void handleFullSyncChunk(const PktFullSyncChunk* pkt, std::vector<SessionEvent>& events);

    PacketHeader buildHeader(PacketType t);
    void         updateAck(uint16_t remoteSeq);

    bool            isHost_;
    State           state_;
    UdpTransport*   transport_;
    uint32_t        seed_;
    int             localWormIdx_;

    // Sequence tracking
    uint16_t        localSeq_;
    uint16_t        remoteSeq_;
    uint16_t        remoteAckBits_;

    // RTT
    int             smoothedRtt_;
    uint32_t        pingId_;
    uint32_t        pingSentMs_;

    // Full-sync reassembly
    struct SyncAssembly {
        int     frame = -1;
        uint32_t totalBytes = 0;
        uint16_t numChunks = 0;
        std::vector<uint8_t> blob;
        std::vector<bool>    received;
        int chunksReceived = 0;
    } syncAssembly_;

    // Wall-clock helper (ms since session created)
    uint32_t nowMs() const;
    uint32_t startMs_;
};
