/* net/udpTransport.cpp */
#include "udpTransport.hpp"

#include <cstdio>
#include <cstring>

UdpTransport::UdpTransport()
: sockValid_(false)
, peerSet_(false)
{
    std::memset(&sock_, 0, sizeof(sock_));
    tl_internet_addr_init_empty(&peer_);
}

UdpTransport::~UdpTransport()
{
    close();
}

bool UdpTransport::bindPort(int port)
{
    if (sockValid_)
    {
        tl_socket_close(sock_);
        sockValid_ = false;
    }

    sock_ = tl_udp_socket();
    if (!tl_socket_is_valid(sock_))
    {
        std::fprintf(stderr, "[udp] Failed to create UDP socket\n");
        return false;
    }
    sockValid_ = true;

    if (tl_bind(sock_, port) < 0)
    {
        std::fprintf(stderr, "[udp] bind failed on port %d\n", port);
        tl_socket_close(sock_);
        sockValid_ = false;
        return false;
    }

    tl_set_nonblocking(sock_, 1);
    return true;
}

bool UdpTransport::setPeer(const char* host, int port)
{
    if (sockValid_)
    {
        tl_socket_close(sock_);
        sockValid_ = false;
    }

    sock_ = tl_udp_socket();
    if (!tl_socket_is_valid(sock_))
    {
        std::fprintf(stderr, "[udp] Failed to create UDP socket\n");
        return false;
    }
    sockValid_ = true;

    // Bind to any local port so we have a source port.
    if (tl_bind(sock_, 0) < 0)
    {
        std::fprintf(stderr, "[udp] bind (any) failed\n");
        tl_socket_close(sock_);
        sockValid_ = false;
        return false;
    }

    tl_internet_addr_init_name(&peer_, host, port);
    peerSet_ = true;
    tl_set_nonblocking(sock_, 1);
    return true;
}

bool UdpTransport::send(const void* buf, size_t len)
{
    if (!peerSet_ || !sockValid_) return false;
    int r = tl_sendto(sock_, buf, (int)len, &peer_);
    return r > 0;
}

int UdpTransport::recv(void* buf, size_t maxLen, tl_internet_addr* srcAddr)
{
    if (!sockValid_) return -1;

    tl_internet_addr tmp;
    tl_internet_addr_init_empty(&tmp);

    int r = tl_recvfrom(sock_, buf, (int)maxLen, &tmp);
    if (r == tl_would_block || r == 0) return 0;
    if (r < 0) return -1;

    if (srcAddr)
        *srcAddr = tmp;
    return r;
}

bool UdpTransport::valid() const
{
    return sockValid_;
}

void UdpTransport::close()
{
    if (sockValid_)
    {
        tl_socket_close(sock_);
        sockValid_ = false;
    }
}
