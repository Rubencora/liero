/* net/packet.hpp — UDP packet format for OpenLiero rollback netcode.
 *
 * All multi-byte fields are little-endian.
 *
 * Header (8 bytes):
 *   magic    : uint8  — always 'L' (0x4C)
 *   type     : uint8  — PacketType enum
 *   seq      : uint16 — sender's packet sequence number
 *   ack      : uint16 — last remote seq ack'd
 *   ackBits  : uint16 — bitmask: bit N = ack-N was received
 *
 * Maximum packet size: 512 bytes (fits in one UDP datagram, well under MTU).
 */
#pragma once

#include <stdint.h>

// ---------------------------------------------------------------------------
// Packet types
// ---------------------------------------------------------------------------
enum PacketType : uint8_t
{
    PT_HandshakeHello  = 0x01, // C → H: "I want to play"
    PT_HandshakeAccept = 0x02, // H → C: "Accepted, here's your seed"
    PT_Input           = 0x10, // bidirectional: frame inputs with FEC
    PT_Ping            = 0x20, // bidirectional: RTT probe
    PT_Pong            = 0x21, // bidirectional: RTT reply
    PT_FullSyncHdr     = 0x30, // H → C: begin full state sync
    PT_FullSyncChunk   = 0x31, // H → C: data chunk
};

// ---------------------------------------------------------------------------
// Wire structures — plain C, no padding issues (all fields hand-packed).
// ---------------------------------------------------------------------------

// Shared 8-byte header, placed at offset 0 in every packet.
struct PacketHeader
{
    uint8_t  magic;    // always 'L'
    uint8_t  type;     // PacketType
    uint16_t seq;      // sender sequence
    uint16_t ack;      // remote ack
    uint16_t ackBits;  // bitfield of ack-1..ack-16
};

static_assert(sizeof(PacketHeader) == 8, "PacketHeader must be 8 bytes");

// PT_HandshakeHello (client → host)
struct PktHandshakeHello
{
    PacketHeader hdr;
    uint16_t     protoVersion; // must match PROTO_VERSION
    uint16_t     _pad;
};

// PT_HandshakeAccept (host → client)
struct PktHandshakeAccept
{
    PacketHeader hdr;
    uint16_t     protoVersion;
    uint16_t     localIdx;   // worm index assigned to the client (0 or 1)
    uint32_t     seed;       // PRNG seed for this match
};

// Per-worm input carried in PT_Input.
// Mirrors the lockstep TCP uint32:
//   bits 0-6  : WormControlStates packed
//   bit  7    : mouse aim active
//   bits 8-14 : mouse aim angle (0-127)
typedef uint32_t NetWormInput;

// PT_Input — carries up to 3 frames of FEC history to survive packet loss.
// history[0] = current frame, history[1] = frame-1, history[2] = frame-2.
struct PktInput
{
    PacketHeader  hdr;
    int32_t       frame;        // simulation frame of history[0]
    NetWormInput  input[3];     // FEC: 3 consecutive frames
};

// PT_Ping / PT_Pong
struct PktPing
{
    PacketHeader hdr;
    uint32_t     pingId;  // echo'd back verbatim in Pong
    uint32_t     tsMs;    // sender timestamp (ms since start)
};
typedef PktPing PktPong;

// PT_FullSyncHdr — announces an upcoming full-state blob.
struct PktFullSyncHdr
{
    PacketHeader hdr;
    int32_t      frame;       // game frame this snapshot was taken at
    uint32_t     totalBytes;  // total uncompressed blob size
    uint16_t     numChunks;   // how many PT_FullSyncChunk packets follow
    uint16_t     _pad;
};

// PT_FullSyncChunk — one fragment of the full-state blob.
#define FULL_SYNC_CHUNK_DATA_MAX 400

struct PktFullSyncChunk
{
    PacketHeader hdr;
    int32_t      frame;       // must match FullSyncHdr.frame
    uint16_t     chunkIdx;    // 0-based
    uint16_t     dataLen;
    uint8_t      data[FULL_SYNC_CHUNK_DATA_MAX];
};

// Convenience: build a header.
inline PacketHeader makeHeader(PacketType t, uint16_t seq, uint16_t ack, uint16_t ackBits)
{
    PacketHeader h;
    h.magic   = 'L';
    h.type    = static_cast<uint8_t>(t);
    h.seq     = seq;
    h.ack     = ack;
    h.ackBits = ackBits;
    return h;
}

// Protocol version — bump whenever packet format changes.
static const uint16_t PROTO_VERSION = 0x0501; // Sprint 5, revision 1
