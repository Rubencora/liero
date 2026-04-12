#ifndef LIERO_GAME_HPP
#define LIERO_GAME_HPP

#include <vector>
#include "level.hpp"
#include "settings.hpp"
#include "weapon.hpp"
#include "sobject.hpp"
#include "nobject.hpp"
#include "bobject.hpp"
#include "rand.hpp"
#include "mixer/player.hpp"
#include "bonus.hpp"
#include "constants.hpp"
#include <string>
#include <gvl/resman/shared_ptr.hpp>
#include "common.hpp"
#include "stats_recorder.hpp"

struct SpectatorViewport;
struct Viewport;
struct Worm;
#ifndef LIERO_HEADLESS
struct Renderer;
#endif

typedef enum {
	StateInitial,
	StateWeaponSelection,
	StateGame,
	StateGameEnded,
} GameState;

struct Holdazone
{
	Holdazone()
	: holderIdx(-1)
	, contenderIdx(-1)
	, contenderFrames(0)
	, timeoutLeft(0)
	, zoneWidth(50), zoneHeight(34)
	{
	}

	gvl::rect rect;
	int holderIdx;

	int contenderIdx, contenderFrames;

	int timeoutLeft;

	int zoneWidth, zoneHeight;
};

struct Game
{
	Game(gvl::shared_ptr<Common> common, gvl::shared_ptr<Settings> settings, gvl::shared_ptr<SoundPlayer> soundPlayer);
	~Game();

	void onKey(uint32_t key, bool state);
	Worm* findControlForKey(uint32_t key, Worm::Control& control);
	void releaseControls();
	void processFrame();
#ifndef LIERO_HEADLESS
	void focus(Renderer& renderer);
	void updateSettings(Renderer& renderer);
#endif

	void createBObject(fixedvec pos, fixedvec vel);
	void createBonus();

	void clearViewports();
	void addViewport(Viewport*);
	void addSpectatorViewport(SpectatorViewport*);
	void processViewports();
#ifndef LIERO_HEADLESS
	void drawViewports(Renderer& renderer, GameState state, bool isReplay = false);
	void drawSpectatorViewports(Renderer& renderer, GameState state, bool isReplay = false);
#endif
	void clearWorms();
	void addWorm(Worm*);
	void resetWorms();
#ifndef LIERO_HEADLESS
	void draw(Renderer& renderer, GameState state, bool useSpectatorViewports, bool isReplay = false);
#endif
	void startGame();
	bool isGameOver();
	void doDamageDirect(Worm& w, int amount, int byIdx);
	void doHealingDirect(Worm& w, int amount);
	void doDamage(Worm& w, int amount, int byIdx);
	void doHealing(Worm& w, int amount);
	void postClone(Game& original, bool complete = false);

	void spawnZone();

	Material pixelMat(int x, int y)
	{
		return common->materials[level.pixel(x, y)];
	}

	Worm* wormByIdx(int idx)
	{
		if (idx < 0) return 0;
		return worms[idx];
	}

	gvl::shared_ptr<Common> common;
	gvl::shared_ptr<SoundPlayer> soundPlayer;
	gvl::shared_ptr<Settings> settings;
	gvl::shared_ptr<StatsRecorder> statsRecorder;

	Level level;

	int screenFlash;
	bool gotChanged;
	int lastKilledIdx;
	bool paused;
	int cycles;
	Rand rand;

	Holdazone holdazone;
	int jugIdx = -1; // GMJuggernaut: index of the current Juggernaut worm

	std::vector<Viewport*> viewports;
	std::vector<SpectatorViewport*> spectatorViewports;
	std::vector<Worm*> worms;

	typedef ExactObjectList<Bonus> BonusList;
	typedef ExactObjectList<WObject> WObjectList;
	typedef ExactObjectList<SObject> SObjectList;
	typedef ExactObjectList<NObject> NObjectList;
	typedef FastObjectList<BObject> BObjectList;
	BonusList bonuses;
	WObjectList wobjects;
	SObjectList sobjects;
	NObjectList nobjects;
	BObjectList bobjects;

	bool quickSim;
};

bool checkRespawnPosition(Game& game, int x2, int y2, int oldX, int oldY, int x, int y);

// 64-bit FNV-1a checksum covering all simulation state (worms, wobjects, nobjects, bonuses, rand, cycles)
uint64_t fullGameChecksum(Game& game);

// Pool scale factor — set before creating any Game (via --pool-scale N flag)
extern int g_poolScale;

// CRC dump flags (set via --dump-crcs / --debug-desync CLI flags)
extern bool g_dumpCrcs;
extern std::string g_dumpCrcsPath;
extern bool g_headless;
extern std::string g_replayPath; // set via --replay <file> to start directly in replay mode

#endif // LIERO_GAME_HPP

