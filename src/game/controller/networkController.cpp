#include "networkController.hpp"
#include "../gfx.hpp"
#include "../worm.hpp"
#include "stats_presenter.hpp"
#include <cstdio>
#include <ctime>

extern "C" {
#include <tl/socket.h>
}

// Reliable send: loops until all bytes are sent or connection drops.
static bool fullSend(tl_socket sock, const void* buf, size_t len)
{
	const char* p = static_cast<const char*>(buf);
	while(len > 0)
	{
		int r = tl_send(sock, p, (int)len);
		if(r <= 0) return false;
		p += r;
		len -= (size_t)r;
	}
	return true;
}

// Reliable recv: loops until all bytes are received or connection drops.
static bool fullRecv(tl_socket sock, void* buf, size_t len)
{
	char* p = static_cast<char*>(buf);
	while(len > 0)
	{
		int r = tl_recv(sock, p, (int)len);
		if(r <= 0) return false;
		p += r;
		len -= (size_t)r;
	}
	return true;
}

NetworkController::NetworkController(
	tl_socket peer,
	int localWormIdx,
	gvl::shared_ptr<Common> common,
	gvl::shared_ptr<Settings> settings)
: LocalController(common, settings)
, peer_(peer)
, localIdx_(localWormIdx)
, remoteIdx_(1 - localWormIdx)
, connected_(true)
{
	// Controls come from keyboard (local) and network (remote) — no AI
	for(auto& w : game.worms)
		w->ai.reset();
}

NetworkController::~NetworkController()
{
	tl_socket_close(peer_);
}

NetworkController* NetworkController::createHost(
	int port,
	gvl::shared_ptr<Common> common,
	gvl::shared_ptr<Settings> settings)
{
	tl_socket listener = tl_tcp_socket();
	if(!tl_socket_is_valid(listener))
	{
		fprintf(stderr, "[net] Failed to create TCP socket\n");
		return nullptr;
	}

	if(tl_bind(listener, port) < 0)
	{
		fprintf(stderr, "[net] bind failed on port %d\n", port);
		tl_socket_close(listener);
		return nullptr;
	}
	tl_listen(listener);

	fprintf(stderr, "[net] Waiting for client on port %d...\n", port);

	tl_internet_addr addr;
	tl_socket peer = tl_accept(listener, &addr);
	tl_socket_close(listener);

	if(!tl_socket_is_valid(peer))
	{
		fprintf(stderr, "[net] accept failed\n");
		return nullptr;
	}
	tl_set_nodelay(peer, 1);

	fprintf(stderr, "[net] Client connected! You control worm 1.\n");

	// Host is worm index 0 (left viewport)
	auto* ctrl = new NetworkController(peer, 0, common, settings);

	// Sync RNG seed: host generates and sends to client
	uint32_t seed = uint32_t(std::time(0));
	ctrl->game.rand.seed(seed);
	if(!fullSend(peer, &seed, sizeof(seed)))
	{
		fprintf(stderr, "[net] Failed to send seed\n");
		delete ctrl;
		return nullptr;
	}

	return ctrl;
}

NetworkController* NetworkController::createClient(
	const std::string& host,
	int port,
	gvl::shared_ptr<Common> common,
	gvl::shared_ptr<Settings> settings)
{
	tl_socket sock = tl_tcp_socket();
	if(!tl_socket_is_valid(sock))
	{
		fprintf(stderr, "[net] Failed to create TCP socket\n");
		return nullptr;
	}

	tl_internet_addr addr;
	tl_internet_addr_init_name(&addr, host.c_str(), port);

	fprintf(stderr, "[net] Connecting to %s:%d...\n", host.c_str(), port);

	if(tl_connect(sock, &addr) < 0)
	{
		fprintf(stderr, "[net] connect failed\n");
		tl_socket_close(sock);
		return nullptr;
	}
	tl_set_nodelay(sock, 1);

	fprintf(stderr, "[net] Connected! You control worm 2.\n");

	// Client is worm index 1 (right viewport)
	auto* ctrl = new NetworkController(sock, 1, common, settings);

	// Sync RNG seed: receive from host and apply
	uint32_t seed = 0;
	if(!fullRecv(sock, &seed, sizeof(seed)))
	{
		fprintf(stderr, "[net] Failed to receive seed\n");
		delete ctrl;
		return nullptr;
	}
	ctrl->game.rand.seed(seed);

	return ctrl;
}

bool NetworkController::syncSend(const void* buf, size_t len)
{
	return fullSend(peer_, buf, len);
}

bool NetworkController::syncRecv(void* buf, size_t len)
{
	return fullRecv(peer_, buf, len);
}

void NetworkController::focus()
{
	if(state == StateGameEnded)
	{
		goingToMenu = true;
		fadeValue = 0;
		return;
	}

	if(state == StateInitial)
	{
		// Skip weapon selection in network mode — go straight to game
		changeState(StateGame);
	}

	game.focus(gfx.playRenderer);
	game.focus(gfx.singleScreenRenderer);
	goingToMenu = false;
	fadeValue = 0;
}

bool NetworkController::process()
{
	if(!connected_)
		return false;

	if(state == StateGame || state == StateGameEnded)
	{
		int realFrameSkip = inverseFrameSkip ? !(cycles % frameSkip) : frameSkip;

		for(int i = 0; i < realFrameSkip && (state == StateGame || state == StateGameEnded); ++i)
		{
			// Lockstep: exchange control states before advancing simulation.
			// Both sides send then receive — same order on each end.
			// Pack local control state + mouse aim into one uint32
			uint32_t myCtrl = game.worms[localIdx_]->controlStates.pack();
			{
				int angle = game.worms[localIdx_]->mouseAimAngle;
				if(angle >= 0)
				{
					myCtrl |= (1u << 7);                          // mouse aim active flag
					myCtrl |= ((uint32_t)(angle & 0x7F) << 8);   // angle in bits 8-14
				}
			}
			uint32_t theirCtrl = 0;

			if(!syncSend(&myCtrl, sizeof(myCtrl)) ||
			   !syncRecv(&theirCtrl, sizeof(theirCtrl)))
			{
				fprintf(stderr, "[net] Connection lost\n");
				connected_ = false;
				goingToMenu = true;
				return true; // let fade-out run
			}

			// Unpack remote control state + mouse aim
			game.worms[remoteIdx_]->controlStates.unpack(theirCtrl);
			if(theirCtrl & (1u << 7))
			{
				game.worms[remoteIdx_]->mouseAimAngle = (int)((theirCtrl >> 8) & 0x7F);
			}
			else
			{
				game.worms[remoteIdx_]->mouseAimAngle = -1;
			}

			game.processFrame();

			if(game.isGameOver())
				changeState(StateGameEnded);
		}
	}

	if(goingToMenu)
	{
		if(fadeValue > 0)
		{
			fadeValue -= 1;
		}
		else
		{
			if(state == StateGameEnded)
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
		if(fadeValue < 33)
			fadeValue += 1;
	}

	return true;
}
