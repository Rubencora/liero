/* net/session.cpp — UDP session implementation. */
#include "session.hpp"

#include <cstdio>
#include <cstring>
#include <chrono>
#include <algorithm>

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
static uint32_t nowMs()
{
    using namespace std::chrono;
    return static_cast<uint32_t>(
        duration_cast<milliseconds>(steady_clock::now().time_since_epoch()).count());
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------
Session::Session(bool isHost, UdpTransport* transport, uint32_t seed, int localIdx)
: isHost_(isHost)
, state_(StateHandshake)
, transport_(transport)
, seed_(seed)
, localWormIdx_(localIdx)
, localSeq_(0)
, remoteSeq_(0)
, remoteAckBits_(0)
, smoothedRtt_(-1)
, pingId_(0)
, pingSentMs_(0)
, startMs_(nowMs())
{}

Session::~Session()
{
    delete transport_;
}

// ---------------------------------------------------------------------------
// Factory methods
// ---------------------------------------------------------------------------
Session* Session::createHost(int port, uint32_t seed)
{
    auto* t = new UdpTransport();
    if (!t->bindPort(port))
    {
        std::fprintf(stderr, "[session] host: bind(%d) failed\n", port);
        delete t;
        return nullptr;
    }
    std::fprintf(stderr, "[session] host: listening on UDP port %d\n", port);
    // Session starts in handshake state; the first datagram will carry
    // the client's Hello. We set hasPeer = false until Hello arrives.
    return new Session(true, t, seed, 0 /*host = worm 0*/);
}

Session* Session::createClient(const std::string& host, int port)
{
    auto* t = new UdpTransport();
    if (!t->setPeer(host.c_str(), port))
    {
        std::fprintf(stderr, "[session] client: setPeer(%s:%d) failed\n",
                     host.c_str(), port);
        delete t;
        return nullptr;
    }

    // Send HandshakeHello
    PktHandshakeHello hello;
    std::memset(&hello, 0, sizeof(hello));
    hello.hdr          = makeHeader(PT_HandshakeHello, 0, 0, 0);
    hello.protoVersion = PROTO_VERSION;
    t->send(&hello, sizeof(hello));

    std::fprintf(stderr, "[session] client: sent Hello to %s:%d\n",
                 host.c_str(), port);

    return new Session(false, t, 0 /*seed from host*/, 1 /*client = worm 1*/);
}

// ---------------------------------------------------------------------------
// poll() — call once per frame
// ---------------------------------------------------------------------------
void Session::poll(std::vector<SessionEvent>& events)
{
    uint8_t buf[512];
    for (;;)
    {
        tl_internet_addr src;
        tl_internet_addr_init_empty(&src);

        int r = transport_->recv(buf, sizeof(buf), &src);
        if (r <= 0) break; // would-block or error

        if (r < (int)sizeof(PacketHeader)) continue; // too small
        const PacketHeader* hdr = reinterpret_cast<const PacketHeader*>(buf);
        if (hdr->magic != 'L') continue; // not ours

        // Accept peer address from first datagram (host mode)
        if (isHost_ && !transport_->hasPeer())
            transport_->setPeerAddr(src);

        updateAck(hdr->seq);
        processPacket(buf, r, src, events);
    }
}

// ---------------------------------------------------------------------------
// Packet dispatch
// ---------------------------------------------------------------------------
void Session::processPacket(const uint8_t* buf, int len,
                             const tl_internet_addr& /*src*/,
                             std::vector<SessionEvent>& events)
{
    const PacketHeader* hdr = reinterpret_cast<const PacketHeader*>(buf);

    switch (static_cast<PacketType>(hdr->type))
    {
    case PT_HandshakeHello:
    {
        if (!isHost_) break; // only host handles Hello
        if (len < (int)sizeof(PktHandshakeHello)) break;
        const PktHandshakeHello* pkt =
            reinterpret_cast<const PktHandshakeHello*>(buf);
        if (pkt->protoVersion != PROTO_VERSION)
        {
            std::fprintf(stderr, "[session] proto version mismatch: got %u want %u\n",
                         pkt->protoVersion, PROTO_VERSION);
            break;
        }
        // Send Accept
        PktHandshakeAccept acc;
        std::memset(&acc, 0, sizeof(acc));
        acc.hdr          = buildHeader(PT_HandshakeAccept);
        acc.protoVersion = PROTO_VERSION;
        acc.localIdx     = 1; // client = worm 1
        acc.seed         = seed_;
        transport_->send(&acc, sizeof(acc));

        state_ = StateConnected;
        SessionEvent ev{};
        ev.type         = SE_Connected;
        ev.seed         = seed_;
        ev.localWormIdx = 0; // host = worm 0
        events.push_back(ev);
        std::fprintf(stderr, "[session] host: handshake complete, seed=%u\n", seed_);
        break;
    }
    case PT_HandshakeAccept:
    {
        if (isHost_) break; // only client handles Accept
        if (len < (int)sizeof(PktHandshakeAccept)) break;
        const PktHandshakeAccept* pkt =
            reinterpret_cast<const PktHandshakeAccept*>(buf);
        if (pkt->protoVersion != PROTO_VERSION) break;

        seed_         = pkt->seed;
        localWormIdx_ = pkt->localIdx; // should be 1

        state_ = StateConnected;
        SessionEvent ev{};
        ev.type         = SE_Connected;
        ev.seed         = seed_;
        ev.localWormIdx = localWormIdx_;
        events.push_back(ev);
        std::fprintf(stderr, "[session] client: handshake complete, seed=%u\n", seed_);
        break;
    }
    case PT_Input:
        if (state_ == StateConnected && len >= (int)sizeof(PktInput))
            handleInput(reinterpret_cast<const PktInput*>(buf), events);
        break;

    case PT_Ping:
        if (len >= (int)sizeof(PktPing))
            handlePing(reinterpret_cast<const PktPing*>(buf));
        break;

    case PT_Pong:
        if (len >= (int)sizeof(PktPong))
            handlePong(reinterpret_cast<const PktPong*>(buf), events);
        break;

    case PT_FullSyncHdr:
        if (state_ == StateConnected && len >= (int)sizeof(PktFullSyncHdr))
            handleFullSyncHdr(reinterpret_cast<const PktFullSyncHdr*>(buf));
        break;

    case PT_FullSyncChunk:
        if (state_ == StateConnected && len >= (int)sizeof(PktFullSyncChunk) - FULL_SYNC_CHUNK_DATA_MAX)
            handleFullSyncChunk(reinterpret_cast<const PktFullSyncChunk*>(buf), events);
        break;

    default:
        break;
    }
}

void Session::handleInput(const PktInput* pkt, std::vector<SessionEvent>& events)
{
    SessionEvent ev{};
    ev.type      = SE_InputArrived;
    ev.numInputs = 0;

    // Decode up to 3 FEC frames (current, current-1, current-2)
    for (int i = 0; i < 3; ++i)
    {
        int frame = pkt->frame - i;
        if (frame < 0) break;
        ev.inputs[ev.numInputs].frame = frame;
        ev.inputs[ev.numInputs].input = pkt->input[i];
        ++ev.numInputs;
    }
    events.push_back(ev);
}

void Session::handlePing(const PktPing* pkt)
{
    // Echo back as Pong
    PktPong pong;
    std::memset(&pong, 0, sizeof(pong));
    pong.hdr    = buildHeader(PT_Pong);
    pong.pingId = pkt->pingId;
    pong.tsMs   = pkt->tsMs;
    transport_->send(&pong, sizeof(pong));
}

void Session::handlePong(const PktPong* pkt, std::vector<SessionEvent>& events)
{
    if (pkt->pingId != pingId_) return; // stale

    uint32_t rtt = nowMs() - pingSentMs_;
    // Exponential moving average: α = 0.25
    if (smoothedRtt_ < 0)
        smoothedRtt_ = static_cast<int>(rtt);
    else
        smoothedRtt_ = (smoothedRtt_ * 3 + static_cast<int>(rtt)) / 4;

    SessionEvent ev{};
    ev.type  = SE_PongArrived;
    ev.rttMs = static_cast<uint32_t>(smoothedRtt_);
    events.push_back(ev);
}

void Session::handleFullSyncHdr(const PktFullSyncHdr* pkt)
{
    syncAssembly_.frame          = pkt->frame;
    syncAssembly_.totalBytes     = pkt->totalBytes;
    syncAssembly_.numChunks      = pkt->numChunks;
    syncAssembly_.blob.assign(pkt->totalBytes, 0);
    syncAssembly_.received.assign(pkt->numChunks, false);
    syncAssembly_.chunksReceived = 0;
}

void Session::handleFullSyncChunk(const PktFullSyncChunk* pkt,
                                   std::vector<SessionEvent>& events)
{
    if (pkt->frame != syncAssembly_.frame) return; // stale/mismatched sync
    if (pkt->chunkIdx >= syncAssembly_.numChunks) return;
    if (syncAssembly_.received[pkt->chunkIdx]) return; // duplicate

    uint32_t offset = static_cast<uint32_t>(pkt->chunkIdx) * FULL_SYNC_CHUNK_DATA_MAX;
    if (offset + pkt->dataLen > syncAssembly_.totalBytes) return;

    std::memcpy(syncAssembly_.blob.data() + offset, pkt->data, pkt->dataLen);
    syncAssembly_.received[pkt->chunkIdx] = true;
    ++syncAssembly_.chunksReceived;

    if (syncAssembly_.chunksReceived == syncAssembly_.numChunks)
    {
        SessionEvent ev{};
        ev.type      = SE_FullSyncReady;
        ev.syncBlob  = std::move(syncAssembly_.blob);
        ev.syncFrame = syncAssembly_.frame;
        events.push_back(ev);
        syncAssembly_.frame = -1;
    }
}

// ---------------------------------------------------------------------------
// Sending
// ---------------------------------------------------------------------------
void Session::sendInput(int frame, const NetWormInput history[3])
{
    PktInput pkt;
    std::memset(&pkt, 0, sizeof(pkt));
    pkt.hdr   = buildHeader(PT_Input);
    pkt.frame = frame;
    pkt.input[0] = history[0];
    pkt.input[1] = history[1];
    pkt.input[2] = history[2];
    transport_->send(&pkt, sizeof(pkt));
}

void Session::sendPing()
{
    pingId_++;
    pingSentMs_ = nowMs();

    PktPing ping;
    std::memset(&ping, 0, sizeof(ping));
    ping.hdr    = buildHeader(PT_Ping);
    ping.pingId = pingId_;
    ping.tsMs   = pingSentMs_;
    transport_->send(&ping, sizeof(ping));
}

void Session::sendFullSync(int frame, const std::vector<uint8_t>& blob)
{
    uint32_t totalBytes = static_cast<uint32_t>(blob.size());
    uint16_t numChunks  = static_cast<uint16_t>(
        (totalBytes + FULL_SYNC_CHUNK_DATA_MAX - 1) / FULL_SYNC_CHUNK_DATA_MAX);

    PktFullSyncHdr hdr;
    std::memset(&hdr, 0, sizeof(hdr));
    hdr.hdr        = buildHeader(PT_FullSyncHdr);
    hdr.frame      = frame;
    hdr.totalBytes = totalBytes;
    hdr.numChunks  = numChunks;
    transport_->send(&hdr, sizeof(hdr));

    for (uint16_t i = 0; i < numChunks; ++i)
    {
        PktFullSyncChunk chunk;
        std::memset(&chunk, 0, sizeof(chunk));
        chunk.hdr      = buildHeader(PT_FullSyncChunk);
        chunk.frame    = frame;
        chunk.chunkIdx = i;

        uint32_t offset  = static_cast<uint32_t>(i) * FULL_SYNC_CHUNK_DATA_MAX;
        uint32_t remain  = totalBytes - offset;
        uint16_t dataLen = static_cast<uint16_t>(
            std::min<uint32_t>(remain, FULL_SYNC_CHUNK_DATA_MAX));
        chunk.dataLen  = dataLen;
        std::memcpy(chunk.data, blob.data() + offset, dataLen);

        transport_->send(&chunk, offsetof(PktFullSyncChunk, data) + dataLen);
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------
PacketHeader Session::buildHeader(PacketType t)
{
    return makeHeader(t, localSeq_++, remoteSeq_, remoteAckBits_);
}

void Session::updateAck(uint16_t remoteSeq)
{
    uint16_t diff = remoteSeq - remoteSeq_;
    if (diff == 0 || diff > 0x8000u) return; // old or same
    // Shift ackBits left by diff, set bit 0 for old remoteSeq_
    remoteAckBits_ = static_cast<uint16_t>(
        (remoteAckBits_ << diff) | (1u << (diff - 1)));
    remoteSeq_ = remoteSeq;
}

uint32_t Session::nowMs() const
{
    using namespace std::chrono;
    return static_cast<uint32_t>(
        duration_cast<milliseconds>(steady_clock::now().time_since_epoch()).count());
}
