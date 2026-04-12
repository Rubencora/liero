/* net/udpTransport.hpp — Thin nonblocking UDP socket wrapper.
 *
 * Wraps tl_udp_socket / tl_sendto / tl_recvfrom.
 * All operations are nonblocking (returns false / -1 on would-block).
 */
#pragma once

#include <stddef.h>
#include <stdbool.h>

extern "C" {
#include <tl/socket.h>
}

struct UdpTransport
{
    UdpTransport();
    ~UdpTransport();

    // Non-copyable
    UdpTransport(const UdpTransport&)            = delete;
    UdpTransport& operator=(const UdpTransport&) = delete;

    // Bind to a local port (host mode). Returns false on error.
    bool bindPort(int port);

    // Set the remote peer address (client mode). Returns false on error.
    bool setPeer(const char* host, int port);

    // For host: extract peer address from the first received packet.
    bool hasPeer() const { return peerSet_; }
    const tl_internet_addr& peer() const { return peer_; }
    void setPeerAddr(const tl_internet_addr& addr) { peer_ = addr; peerSet_ = true; }

    // Send buf[0..len) to peer. Returns true on success.
    bool send(const void* buf, size_t len);

    // Receive up to maxLen bytes into buf. Returns number of bytes read,
    // 0 on would-block, -1 on error. Fills srcAddr if non-null.
    int recv(void* buf, size_t maxLen, tl_internet_addr* srcAddr = nullptr);

    bool valid() const;
    void close();

private:
    tl_socket sock_;
    bool sockValid_;       // separate flag avoids tl_socket_invalid() (Unix name mismatch)
    tl_internet_addr peer_;
    bool peerSet_;
};
