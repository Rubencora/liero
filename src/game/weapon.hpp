#ifndef LIERO_WEAPON_HPP
#define LIERO_WEAPON_HPP

#include "math.hpp"
#include "exactObjectList.hpp"
#include <string>

struct Worm;
struct Game;
struct Settings;
struct WormWeapon;

struct Weapon
{
	enum
	{
		STNormal,
		STDType1,
		STSteerable,
		STDType2,
		STLaser,
		STHoming
	};

	void fire(Game& game, int angle, fixedvec vel, int speed, fixedvec pos, int ownerIdx, WormWeapon* ww) const;

	int detectDistance;
	bool affectByWorm;
	int blowAway;
	fixed gravity;
	bool shadow;
	bool laserSight;
	int launchSound;
	int loopSound;
	int exploSound;
	int speed;
	fixed addSpeed;
	int distribution;
	int parts;
	int recoil;
	int multSpeed;
	int delay;
	int loadingTime;
	int ammo;
	int createOnExp;
	int dirtEffect;
	int leaveShells;
	int leaveShellDelay;
	bool playReloadSound;
	bool wormExplode;
	bool explGround;
	bool wormCollide;
	int fireCone;
	bool collideWithObjects;
	bool affectByExplosions;
	int bounce;
	int timeToExplo;
	int timeToExploV;
	int hitDamage;
	int bloodOnHit;
	int startFrame;
	int numFrames;
	bool loopAnim;
	int shotType;
	int colorBullets;
	int splinterAmount;
	int splinterColour;
	int splinterType;
	int splinterScatter;
	int objTrailType;
	int objTrailDelay;
	int partTrailType;
	int partTrailObj;
	int partTrailDelay;
	bool chainExplosion;

	// Sprint 7 engine hooks (all default 0/false so old .cfg files are unaffected)
	int  homingStrength      = 0;     // angle steps/frame to steer toward nearest worm (0=off)
	int  attractRadius       = 0;     // pixels; if >0 pull nearby objects toward projectile each frame
	int  attractForce        = 0;     // fixed-point force magnitude (e.g. 800)
	int  chainLightningJumps = 0;     // on worm hit: jump to nearest other worm ≤80px, up to N times
	bool onExpireTeleport    = false; // on timeLeft expire: teleport owner worm to projectile pos
	bool dirtDeposit         = false; // blowUpObject: add dirt to background pixels instead of removing
	bool pierceDirt          = false; // do not collide with terrain; pass straight through

	int computedLoadingTime(Settings& settings) const;

	int id;
	std::string name;
	std::string idStr;
};

struct WObject : ExactObjectListBase
{
	void blowUpObject(Game& game, int causeIdx);
	void process(Game& game);

	fixedvec pos, vel;
	//int id;
	Weapon const* type;
	int ownerIdx;
	int curFrame;
	int timeLeft;

	// STATS
	WormWeapon* firedBy;
	bool hasHit;
};


#endif // LIERO_WEAPON_HPP
