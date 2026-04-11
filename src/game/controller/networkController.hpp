#ifndef LIERO_CONTROLLER_NETWORK_CONTROLLER_HPP
#define LIERO_CONTROLLER_NETWORK_CONTROLLER_HPP

#include "localController.hpp"
#include <string>

extern "C" {
#include <tl/socket.h>
}

struct NetworkController : LocalController
{
	// Create host: listens on port, blocks until client connects.
	// Worm 0 = local (host), Worm 1 = remote (client).
	static NetworkController* createHost(
		int port,
		gvl::shared_ptr<Common> common,
		gvl::shared_ptr<Settings> settings);

	// Create client: connects to host.
	// Worm 0 = remote (host), Worm 1 = local (client).
	static NetworkController* createClient(
		const std::string& host,
		int port,
		gvl::shared_ptr<Common> common,
		gvl::shared_ptr<Settings> settings);

	~NetworkController();

	void focus() override;
	bool process() override;

private:
	NetworkController(
		tl_socket peer,
		int localWormIdx,
		gvl::shared_ptr<Common> common,
		gvl::shared_ptr<Settings> settings);

	bool syncSend(const void* buf, size_t len);
	bool syncRecv(void* buf, size_t len);

	tl_socket peer_;
	int localIdx_;   // which worm we control via keyboard
	int remoteIdx_;  // which worm we receive via network
	bool connected_;
};

#endif // LIERO_CONTROLLER_NETWORK_CONTROLLER_HPP
