
#include "localController.hpp"

#include <gvl/system/system.hpp>
#include <gvl/io2/fstream.hpp>
#include "stats_presenter.hpp"
#include "../keys.hpp"
#include "../gfx.hpp"
#include "../sfx.hpp"
#include "../reader.hpp"
#include "../filesystem.hpp"
#include "../replay.hpp"

#include "../ai/predictive_ai.hpp"
#include "../worm.hpp"
#include "../spectatorviewport.hpp"
#include "../viewport.hpp"

#include <cctype>
#include <cmath>
#include <fstream>

gvl::shared_ptr<WormAI> createAi(int controller, Worm& worm, Settings& settings)
{
	if (controller == 1)
		return gvl::shared_ptr<WormAI>(new DumbLieroAI());
	else if (controller == 2)
		return gvl::shared_ptr<WormAI>(new FollowAI(
			Weights(), settings.aiParallels, worm.index == 0));

	return gvl::shared_ptr<WormAI>();
}

// Compute the HUD statsX for worm i out of n total worms.
// For 2p, preserve the original offsets (0 and 218).
static int hudStatsX(int i, int n)
{
	if (n <= 2)
	{
		static const int xs2[2] = {0, 218};
		return xs2[i < 2 ? i : 1];
	}
	if (n == 3)
	{
		static const int xs3[3] = {0, 107, 214};
		return xs3[i < 3 ? i : 2];
	}
	// 4 players: 78px gaps, bars are 75px wide → rightmost ends at 234+75=309 < 320.
	static const int xs4[4] = {0, 78, 156, 234};
	return xs4[i < 4 ? i : 3];
}

// Create viewports for n worms within the 320x200 canvas.
// Viewport area is 320x161 (rows 0-160); HUD occupies rows 161-199.
// 2-player: two 158x158 side-by-side (unchanged).
// 3-4 player: 2x2 grid of 158x78 each.
static void makeViewports(Game& game, int n)
{
	switch (n)
	{
	case 2:
	default:
		// Identical to the original layout — must not change for 2p.
		game.addViewport(new Viewport(gvl::rect(  0,  0, 158, 158), 0, 504, 350));
		game.addViewport(new Viewport(gvl::rect(160,  0, 318, 158), 1, 504, 350));
		break;
	case 3:
		game.addViewport(new Viewport(gvl::rect(  0,  0, 158,  78), 0, 504, 350));
		game.addViewport(new Viewport(gvl::rect(160,  0, 318,  78), 1, 504, 350));
		game.addViewport(new Viewport(gvl::rect(  0, 80, 158, 158), 2, 504, 350));
		break;
	case 4:
		game.addViewport(new Viewport(gvl::rect(  0,  0, 158,  78), 0, 504, 350));
		game.addViewport(new Viewport(gvl::rect(160,  0, 318,  78), 1, 504, 350));
		game.addViewport(new Viewport(gvl::rect(  0, 80, 158, 158), 2, 504, 350));
		game.addViewport(new Viewport(gvl::rect(160, 80, 318, 158), 3, 504, 350));
		break;
	}
}

LocalController::LocalController(gvl::shared_ptr<Common> common, gvl::shared_ptr<Settings> settings)
: game(common, settings, gvl::shared_ptr<SoundPlayer>(new DefaultSoundPlayer(*common)))
, state(StateInitial)
, fadeValue(0)
, goingToMenu(false)
{
	int n = settings->numPlayers;
	if (n < 2) n = 2;
	if (n > 4) n = 4;
	settings->ensureWormCount(n);

	for (int i = 0; i < n; ++i)
	{
		Worm* w = new Worm();
		w->settings = settings->wormSettings[i];
		w->health   = w->settings->health;
		w->index    = i;
		w->statsX   = hudStatsX(i, n);
		w->ai       = createAi(w->settings->controller, *w, *settings);
		game.addWorm(w);
	}

	makeViewports(game, n);

	// +68 on x to align the spectator viewport in the middle
	game.addSpectatorViewport(new SpectatorViewport(gvl::rect(0, 0, 504 + 68, 350), 504, 350));
}

LocalController::~LocalController()
{
	endRecord();
}

void LocalController::onKey(int key, bool keyState)
{
	Worm::Control control;
	Worm* worm = game.findControlForKey(key, control);
	if(worm)
	{
		worm->cleanControlStates.set(control, keyState);

		if(control < Worm::MaxControl)
		{
			// Only real controls
			worm->setControlState(control, keyState);
		}

		if(worm->cleanControlStates[WormSettings::Dig])
		{
			worm->press(Worm::Left);
			worm->press(Worm::Right);
		}
		else
		{
			if(!worm->cleanControlStates[Worm::Left])
				worm->release(Worm::Left);
			if(!worm->cleanControlStates[Worm::Right])
				worm->release(Worm::Right);
		}
	}

	if(key == DkEscape && !goingToMenu)
	{
		fadeValue = 31;
		goingToMenu = true;
	}

	// F9 (DOS scan 0x43 = 67) toggles mouse aim
	if(key == 67 && keyState)
	{
		mouseAimEnabled_ = !mouseAimEnabled_;
	}
}

// Called when the controller loses focus. When not focused, it will not receive key events among other things.
void LocalController::unfocus()
{
	if(replay.get())
		replay->unfocus();
	if(state == StateWeaponSelection)
		ws->unfocus();
}

// Called when the controller gets focus.
void LocalController::focus()
{
	if(state == StateGameEnded)
	{
		goingToMenu = true;
		fadeValue = 0;
		return;
	}
	if(state == StateWeaponSelection)
		ws->focus();
	if(replay.get())
		replay->focus();
	if(state == StateInitial)
		changeState(StateWeaponSelection);
	game.focus(gfx.playRenderer);
	// FIXME rewrite the focus function to avoid nonsense like this?
	game.focus(gfx.singleScreenRenderer);
	goingToMenu = false;
	fadeValue = 0;
}

bool LocalController::process()
{
	if(state == StateWeaponSelection)
	{
		if(ws->processFrame())
			changeState(StateGame);
	}
	else if(state == StateGame || state == StateGameEnded)
	{
		int realFrameSkip = inverseFrameSkip ? !(cycles % frameSkip) : frameSkip;
		for(int i = 0; i < realFrameSkip && (state == StateGame || state == StateGameEnded); ++i)
		{
			int phase = game.cycles % 2;
			for (std::size_t i = 0; i < game.worms.size(); ++i)
			{
				Worm& worm = *game.worms[(i + phase) % game.worms.size()];
				if(worm.ai.get())
				{
					uint64_t time = gvl::get_hires_ticks();
					worm.ai->process(game, worm);
					time = gvl::get_hires_ticks() - time;
					game.statsRecorder->aiProcessTime(&worm, time);
				}
			}
			if(replay.get())
			{
				try
				{
					replay->recordFrame();
				}
				catch(std::runtime_error& e)
				{
					Console::writeWarning(std::string("Error recording replay frame: ") + e.what());
					Console::writeWarning("Replay recording aborted");
					replay.reset();
				}
			}

			// Mouse aim: sample cursor once per game frame
			if(state == StateGame && mouseAimEnabled_ && gfx.sdlRenderer)
			{
				int raw_mx, raw_my;
				SDL_GetMouseState(&raw_mx, &raw_my);
				float lx, ly;
				SDL_RenderWindowToLogical(gfx.sdlRenderer, raw_mx, raw_my, &lx, &ly);
				// lx,ly are already in the 320x200 logical space

				for(std::size_t wi = 0; wi < game.worms.size(); ++wi)
				{
					Worm& worm = *game.worms[wi];
					if(worm.ai.get() || !worm.visible) continue;
					if(worm.settings->controller != 0) continue;

					// Find this worm's viewport
					Viewport* vp = nullptr;
					for(auto* v : game.viewports)
					{
						if(v->wormIdx == (int)wi) { vp = v; break; }
					}
					if(!vp) continue;

					int worm_sx = ftoi(worm.pos.x) - vp->x + vp->rect.x1;
					int worm_sy = ftoi(worm.pos.y) - vp->y + vp->rect.y1;

					float dx = lx - (float)worm_sx;
					float dy = ly - (float)worm_sy;

					// Only override direction when cursor is not too close
					if(std::fabs(dx) > 2.0f)
						worm.direction = (dx > 0) ? 1 : 0;

					// Map angle: 64=horizontal, 12=upper-left, 116=lower-right
					float dx_front = (worm.direction == 1) ? dx : -dx;
					float angle_rad = std::atan2f(dy, std::max(dx_front, 1.0f));
					// atan2 range: -pi/2 to pi/2 when dx_front>0
					// liero range right: 64 (horiz) to 116 (down) to 64 (horiz up via 64..116)
					// 64 = horizontal (angle_rad=0), 116 = straight down (angle_rad=pi/2)
					int liero_angle = 64 + (int)std::roundf(angle_rad * 52.0f / (float)(M_PI * 0.5));
					liero_angle = std::max(12, std::min(116, liero_angle));

					worm.mouseAimAngle = liero_angle;
				}
			}
			else if(!mouseAimEnabled_)
			{
				for(std::size_t wi = 0; wi < game.worms.size(); ++wi)
					game.worms[wi]->mouseAimAngle = -1;
			}

			game.processFrame();

			if(g_dumpCrcs)
			{
				static std::ofstream crcFile;
				if(!crcFile.is_open())
					crcFile.open(g_dumpCrcsPath, std::ios::out | std::ios::trunc);
				if(crcFile.is_open())
				{
					uint64_t crc = fullGameChecksum(game);
					crcFile << game.cycles << "," << crc << "\n";
				}
			}

			if(game.isGameOver())
			{
				changeState(StateGameEnded);
			}
		}
	}

	//CommonController::process();

	if(goingToMenu)
	{
		if(fadeValue > 0)
			fadeValue -= 1;
		else
		{
			if(state == StateGameEnded)
			{
				endRecord();
				game.statsRecorder->finish(game);
				// TODO: Get rid of cast.
				presentStats(static_cast<NormalStatsRecorder&>(*game.statsRecorder), game);
			}
			return false;
		}
	}
	else
	{
		if(fadeValue < 33)
		{
			fadeValue += 1;
		}
	}

	return true;
}

void LocalController::draw(Renderer& renderer, bool useSpectatorViewports)
{
	if(state == StateWeaponSelection)
	{
		ws->draw(renderer, state, useSpectatorViewports);
	}
	else if(state == StateGame || state == StateGameEnded || state == StateInitial)
	{
		game.draw(renderer, state, useSpectatorViewports);
	}
	renderer.fadeValue = fadeValue;
}

void LocalController::changeState(GameState newState)
{
	if(state == newState)
		return;

	// NOTE: We prepare new state before destroying the old.
	// e.g. weapon selection is destroyed first after we successfully
	// started recording.

	// NOTE: Must do this here before starting recording!
	if(state == StateWeaponSelection)
	{
		ws->finalize();
	}

	if(newState == StateWeaponSelection)
	{
		ws.reset(new WeaponSelection(game));
	}
	else if(newState == StateGame)
	{
		// NOTE: This must be done before the replay recording starts below
		for(std::size_t i = 0; i < game.worms.size(); ++i)
		{
			Worm& worm = *game.worms[i];
			worm.lives = game.settings->lives;
		}

		if(game.settings->extensions && game.settings->recordReplays)
		{
			try
			{
#if !ENABLE_TRACING
				std::time_t ticks = std::time(0);
				std::tm* now = std::localtime(&ticks);

				char buf[512];
				std::strftime(buf, sizeof(buf), "%Y-%m-%d %H.%M.%S", now);

				std::string playerNames = " ";
				for(std::size_t i = 0; i < game.worms.size(); ++i)
				{
					Worm& worm = *game.worms[i];
					std::string const& name = worm.settings->name;
					int chars = 0;

					if(i > 0)
						playerNames.push_back('-');
					for(std::size_t c = 0; c < name.size() && chars < 4; ++c, ++chars)
					{
						unsigned char ch = (unsigned char)name[c];
						if(std::isalnum(ch))
							playerNames.push_back(ch);
					}
				}
#else
				std::string prefix = "-  Trace";
				std::string buf = ".lrp";
#endif
				//std::string path = joinPath(joinPath(configRoot, "Replays"), prefix + buf);
				//create_directories(path);

				auto node = gfx.getConfigNode() / "Replays" / (buf + playerNames + ".lrp");

				replay.reset(new ReplayWriter(node.toSink()));

				//replay.reset(new ReplayWriter(gvl::sink(new gvl::file_bucket_pipe(path.c_str(), "wb"))));
				replay->beginRecord(game);
			}
			catch(std::runtime_error& e)
			{
				gfx.infoBox(std::string("Error starting replay recording: ") + e.what());
				goingToMenu = true;
				fadeValue = 0;
				return;
			}
		}

		game.startGame();
	}
	else if(newState == StateGameEnded)
	{
		if(!goingToMenu)
		{
			fadeValue = 180;
			goingToMenu = true;
		}
	}

	if(state == StateWeaponSelection)
	{
		fadeValue = 33;
		ws.reset();
	}

	state = newState;
}

void LocalController::endRecord()
{
	if(replay.get())
	{
		replay.reset();
	}
}

void LocalController::swapLevel(Level& newLevel)
{
	currentLevel()->swap(newLevel);
}

Level* LocalController::currentLevel()
{
	return &game.level;
}

Game* LocalController::currentGame()
{
	return &game;
}

bool LocalController::running()
{
	return state != StateGameEnded && state != StateInitial;
}
