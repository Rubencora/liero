#include "weapon.hpp"
#include "game.hpp"
#include "mixer/player.hpp"
#include "math.hpp"
#include "gfx/blit.hpp"
#include "constants.hpp"
#include <algorithm>
#include <climits>

int Weapon::computedLoadingTime(Settings& settings) const
{
	int ret = (settings.loadingTime * loadingTime) / 100;
	if(ret == 0)
		ret = 1;
	return ret;
}

void Weapon::fire(Game& game, int angle, fixedvec vel, int speed, fixedvec pos, int ownerIdx, WormWeapon* ww) const
{
	WObject* obj = game.wobjects.newObjectReuse();
	IF_ENABLE_TRACING(Common& common = *game.common);

	obj->type = this;
	obj->pos = pos;
	obj->ownerIdx = ownerIdx;

	// STATS
	obj->firedBy = ww;
	obj->hasHit = false;

	LTRACE(rand, 0, wobj, game.rand.x);
	LTRACE(fire, obj - game.wobjects.data(), cxpo, pos.x);
	LTRACE(fire, obj - game.wobjects.data(), cypo, pos.y);

	Worm* owner = game.wormByIdx(ownerIdx);
	game.statsRecorder->damagePotential(owner, ww, hitDamage);
	game.statsRecorder->shot(owner, ww);

	obj->vel = cossinTable[angle] * speed / 100 + vel;

	if(distribution)
	{
		obj->vel.x += game.rand(distribution * 2) - distribution;
		obj->vel.y += game.rand(distribution * 2) - distribution;
	}

	if(startFrame >= 0)
	{
		if(shotType == STNormal)
		{
			if(loopAnim)
			{
				if(numFrames)
					obj->curFrame = game.rand(numFrames + 1);
				else
					obj->curFrame = game.rand(2);
			}
			else
				obj->curFrame = 0;
		}
		else if(shotType == STDType1)
		{
			if(angle > 64)
				--angle;

			int curFrame = (angle - 12) >> 3;
			if(curFrame < 0)
				curFrame = 0;
			else if(curFrame > 12)
				curFrame = 12;
			obj->curFrame = curFrame;
		}
		else if(shotType == STDType2 || shotType == STSteerable || shotType == STHoming)
		{
			obj->curFrame = angle;
		}
		else
			obj->curFrame = 0;
	}
	else
	{
		obj->curFrame = colorBullets - game.rand(2);
	}

	obj->timeLeft = timeToExplo;

	if(timeToExploV)
		obj->timeLeft -= game.rand(timeToExploV);
}

void WObject::blowUpObject(Game& game, int causeIdx)
{
	Common& common = *game.common;
	Weapon const& w = *type;

	fixed x = this->pos.x;
	fixed y = this->pos.y;
	fixed velX = this->vel.x;
	fixed velY = this->vel.y;

	game.wobjects.free(this);

	if(w.createOnExp >= 0)
	{
		common.sobjectTypes[w.createOnExp].create(game, ftoi(x), ftoi(y), causeIdx, firedBy, this);
	}

	if(w.exploSound >= 0)
	{
		game.soundPlayer->play(w.exploSound);
	}

	int splinters = w.splinterAmount;

	if(splinters > 0)
	{
		if(w.splinterScatter == 0)
		{
			for(int i = 0; i < splinters; ++i)
			{
				int angle = game.rand(128);
				int colorSub = game.rand(2);
				common.nobjectTypes[w.splinterType].create2(
					game,
					angle,
					fixedvec(),
					fixedvec(x, y),
					w.splinterColour - colorSub,
					causeIdx,
					firedBy);
			}
		}
		else
		{
			for(int i = 0; i < splinters; ++i)
			{
				int colorSub = game.rand(2);
				common.nobjectTypes[w.splinterType].create1(
					game,
					fixedvec(velX, velY),
					fixedvec(x, y),
					w.splinterColour - colorSub,
					causeIdx,
					firedBy);
			}
		}
	}

	if(w.dirtDeposit)
	{
		int ix = ftoi(x), iy = ftoi(y);
		drawDirtDeposit(common, game.level, ix - 8, iy - 12, 16, 24);
		if(game.settings->shadow)
			correctShadow(common, game.level, gvl::rect(ix - 12, iy - 16, ix + 13, iy + 13));
	}
	else if(w.dirtEffect >= 0)
	{
		int ix = ftoi(x), iy = ftoi(y);
		drawDirtEffect(common, game.rand, game.level, w.dirtEffect, ftoi(x) - 7, ftoi(y) - 7);
		if(game.settings->shadow)
			correctShadow(common, game.level, gvl::rect(ix - 10, iy - 10, ix + 11, iy + 11));
	}
}

void WObject::process(Game& game)
{
	int iter = 0;
	bool doExplode = false;
	bool doRemove = false;

	Common& common = *game.common;
	Weapon const& w = *type;

	Worm* owner = game.wormByIdx(ownerIdx);

	// As liero would do this while rendering, we try to do it as early as possible
	if(common.H[HRemExp]
	&& type - &common.weapons[0] == LC(RemExpObject) - 1)
	{
		if(owner->pressed(Worm::Change)
		&& owner->pressed(Worm::Fire))
		{
			timeLeft = 0;
		}
	}

	do
	{
		++iter;
		pos += vel;

		if(w.shotType == 2)
		{
			fixedvec dir(cossinTable[curFrame]);
			auto newVel = dir * w.speed / 100;

			if(owner->visible
			&& owner->pressed(Worm::Up))
			{
				newVel += dir * w.addSpeed / 100;
			}

			vel = ((vel * 8) + newVel) / 9;
		}
		else if(w.shotType == 3)
		{
			fixedvec dir(cossinTable[curFrame]);
			auto addVel = dir * w.addSpeed / 100;

			vel += addVel;

			if(w.distribution)
			{
				vel.x += game.rand(w.distribution * 2) - w.distribution;
				vel.y += game.rand(w.distribution * 2) - w.distribution;
			}
		}
		else if(w.shotType == Weapon::STHoming && w.homingStrength > 0)
		{
			// Find nearest visible enemy worm
			Worm* target = nullptr;
			int minDist2 = INT_MAX;
			for(std::size_t wi = 0; wi < game.worms.size(); ++wi)
			{
				Worm* cand = game.worms[wi];
				if((int)wi == ownerIdx || !cand->visible) continue;
				int dx = ftoi(cand->pos.x) - ftoi(pos.x);
				int dy = ftoi(cand->pos.y) - ftoi(pos.y);
				int d2 = dx*dx + dy*dy;
				if(d2 < minDist2) { minDist2 = d2; target = cand; }
			}

			if(target)
			{
				int tx = ftoi(target->pos.x) - ftoi(pos.x);
				int ty = ftoi(target->pos.y) - ftoi(pos.y);
				// Cross product of vel and target direction (sign tells steer direction)
				long long cross = (long long)vel.x * ty - (long long)vel.y * tx;
				for(int step = 0; step < w.homingStrength; ++step)
				{
					if(cross < 0)
						curFrame = (curFrame + 1) & 127;
					else if(cross > 0)
						curFrame = (curFrame + 127) & 127;
				}
				// Set velocity from new angle
				vel = cossinTable[curFrame] * w.speed / 100;
			}
		}

		// Magnet: attract nearby wobjects, nobjects, and worms toward this projectile.
		// Pull = attractForce/1000 pixels/frame (constant direction, not distance-scaled).
		if(w.attractRadius > 0 && w.attractForce > 0)
		{
			auto wr = game.wobjects.all();
			for(WObject* i; (i = wr.next()); )
			{
				if(i == this) continue;
				int dx = ftoi(pos.x) - ftoi(i->pos.x);
				int dy = ftoi(pos.y) - ftoi(i->pos.y);
				int dist = vectorLength(dx, dy);
				if(dist > 0 && dist < w.attractRadius)
				{
					i->vel.x += (fixed)((long long)itof(dx) * w.attractForce / (1000 * dist));
					i->vel.y += (fixed)((long long)itof(dy) * w.attractForce / (1000 * dist));
				}
			}

			auto nr = game.nobjects.all();
			for(NObject* i; (i = nr.next()); )
			{
				int dx = ftoi(pos.x) - ftoi(i->pos.x);
				int dy = ftoi(pos.y) - ftoi(i->pos.y);
				int dist = vectorLength(dx, dy);
				if(dist > 0 && dist < w.attractRadius)
				{
					i->vel.x += (fixed)((long long)itof(dx) * w.attractForce / (1000 * dist));
					i->vel.y += (fixed)((long long)itof(dy) * w.attractForce / (1000 * dist));
				}
			}

			for(std::size_t wi = 0; wi < game.worms.size(); ++wi)
			{
				Worm* wm = game.worms[wi];
				if(!wm->visible) continue;
				int dx = ftoi(pos.x) - ftoi(wm->pos.x);
				int dy = ftoi(pos.y) - ftoi(wm->pos.y);
				int dist = vectorLength(dx, dy);
				if(dist > 0 && dist < w.attractRadius)
				{
					wm->vel.x += (fixed)((long long)itof(dx) * w.attractForce / (1000 * dist));
					wm->vel.y += (fixed)((long long)itof(dy) * w.attractForce / (1000 * dist));
				}
			}
		}

		if(w.bounce > 0)
		{
			auto ipos = ftoi(pos);
			auto inewPos = ftoi(pos + vel);

			if(!game.level.inside(inewPos.x, ipos.y)
			|| game.pixelMat(inewPos.x, ipos.y).dirtRock())
			{
				if(w.bounce != 100)
				{
					vel.x = -vel.x * w.bounce / 100;
					vel.y = (vel.y * 4) / 5; // TODO: Read from EXE
				}
				else
					vel.x = -vel.x;
			}

			if(!game.level.inside(ipos.x, inewPos.y)
			|| game.pixelMat(ipos.x, inewPos.y).dirtRock())
			{
				if(w.bounce != 100)
				{
					vel.y = -vel.y * w.bounce / 100;
					vel.x = (vel.x * 4) / 5; // TODO: Read from EXE
				}
				else
					vel.y = -vel.y;
			}
		}

		if(w.multSpeed != 100)
		{
			vel = vel * w.multSpeed / 100;
		}

		if(w.objTrailType >= 0 && (game.cycles % w.objTrailDelay) == 0)
		{
			common.sobjectTypes[w.objTrailType].create(game, ftoi(pos.x), ftoi(pos.y), ownerIdx, firedBy);
		}

		if(w.partTrailObj >= 0 && (game.cycles % w.partTrailDelay) == 0)
		{
			if(w.partTrailType == 1)
			{
				common.nobjectTypes[w.partTrailObj].create1(
					game,
					vel / LC(SplinterLarpaVelDiv),
					pos,
					0,
					ownerIdx,
					firedBy);
			}
			else
			{
				int angle = game.rand(128);
				common.nobjectTypes[w.partTrailObj].create2(
					game,
					angle,
					vel / LC(SplinterCracklerVelDiv),
					pos,
					0,
					ownerIdx,
					firedBy);
			}
		}

		if(w.collideWithObjects)
		{
			auto impulse = vel * w.blowAway / 100;

			auto wr = game.wobjects.all();
			for (WObject* i; (i = wr.next()); )
			{
				if(i->type != type
				|| i->ownerIdx != ownerIdx)
				{
					if(pos.x >= i->pos.x - itof(2)
					&& pos.x <= i->pos.x + itof(2)
					&& pos.y >= i->pos.y - itof(2)
					&& pos.y <= i->pos.y + itof(2))
					{
						i->vel += impulse;
					}
				}
			}

			auto nr = game.nobjects.all();
			for (NObject* i; (i = nr.next()); )
			{
				if(pos.x >= i->pos.x - itof(2)
				&& pos.x <= i->pos.x + itof(2)
				&& pos.y >= i->pos.y - itof(2)
				&& pos.y <= i->pos.y + itof(2))
				{
					i->vel += impulse;
				}
			}
		}

		auto inewPos = ftoi(pos + vel);

		if(inewPos.x < 0)
			pos.x = 0;
		if(inewPos.y < 0)
			pos.y = 0;
		if(inewPos.x >= game.level.width)
			pos.x = itof(game.level.width - 1);
		if(inewPos.y >= game.level.height)
			pos.y = itof(game.level.height - 1);

		if(!game.level.inside(inewPos)
		|| game.pixelMat(inewPos.x, inewPos.y).dirtRock())
		{
			if(w.pierceDirt && game.level.inside(inewPos))
			{
				// Pass through terrain — do nothing
			}
			else if(w.bounce == 0)
			{
				if(w.explGround)
				{
					doExplode = true;
				}
				else
				{
					vel.zero();
				}
			}
		}
		else
		{
			vel.y += w.gravity; // The original tested w.gravity first, which doesn't seem like a gain

			if(w.numFrames > 0)
			{
				if((game.cycles & 7) == 0)
				{
					if(!w.loopAnim)
					{
						if(++curFrame > w.numFrames)
							curFrame = 0;
					}
					else
					{
						if(vel.x < 0)
						{
							if(--curFrame < 0)
								curFrame = w.numFrames;
						}
						else if(vel.x > 0)
						{
							if(++curFrame > w.numFrames)
								curFrame = 0;
						}
					}
				}
			}
		}

		if(w.timeToExplo > 0)
		{
			if(--timeLeft < 0)
			{
				if(w.onExpireTeleport && owner && owner->visible)
				{
					// Teleport owner worm to projectile position
					owner->pos = pos;
					owner->vel = fixedvec();
					game.wobjects.free(this);
					return;
				}
				doExplode = true;
			}
		}

		for(std::size_t i = 0; i < game.worms.size(); ++i)
		{
			Worm& worm = *game.worms[i];

			if((w.hitDamage || w.blowAway || w.bloodOnHit || w.wormCollide)
			&& checkForSpecWormHit(game, ftoi(pos.x), ftoi(pos.y), w.detectDistance, worm))
			{
				worm.vel += vel * w.blowAway / 100;

				game.doDamage(worm, w.hitDamage, ownerIdx);
				game.statsRecorder->damageDealt(owner, firedBy, &worm, w.hitDamage, hasHit);
				if (!hasHit)
					game.statsRecorder->hit(owner, firedBy, &worm);
				hasHit = true;

				int bloodAmount = w.bloodOnHit * game.settings->blood / 100;

				for(int i = 0; i < bloodAmount; ++i)
				{
					int angle = game.rand(128);
					common.nobjectTypes[6].create2(game, angle, vel / 3, pos, 0, worm.index, firedBy);
				}

				if(w.hitDamage > 0
				&& worm.health > 0
				&& game.rand(3) == 0)
				{
					int snd = game.rand(3) + 18; // NOTE: MUST be outside the unpredictable branch below
					if(!game.soundPlayer->isPlaying(&worm))
					{
						game.soundPlayer->play(snd, &worm);
					}
				}

				// Chain Lightning: jump to nearest other worm within 80px
				if(w.chainLightningJumps > 0 && w.hitDamage > 0)
				{
					int jumpsLeft = w.chainLightningJumps;
					int curDmg = w.hitDamage;
					Worm* lastHit = &worm;
					while(jumpsLeft > 0)
					{
						curDmg = std::max(1, curDmg / 2);
						Worm* nextTarget = nullptr;
						int minD2 = 80*80;
						for(std::size_t ji = 0; ji < game.worms.size(); ++ji)
						{
							Worm* cand = game.worms[ji];
							if(cand == lastHit || !cand->visible) continue;
							int cdx = ftoi(cand->pos.x) - ftoi(lastHit->pos.x);
							int cdy = ftoi(cand->pos.y) - ftoi(lastHit->pos.y);
							int cd2 = cdx*cdx + cdy*cdy;
							if(cd2 < minD2) { minD2 = cd2; nextTarget = cand; }
						}
						if(!nextTarget) break;
						game.doDamage(*nextTarget, curDmg, ownerIdx);
						lastHit = nextTarget;
						--jumpsLeft;
					}
				}

				if(w.wormCollide)
				{
					if(game.rand(w.wormCollide) == 0)
					{
						if(w.wormExplode)
							doExplode = true;

						doRemove = true;
					}
				}
			}
		}

		if(doExplode)
		{
			blowUpObject(game, ownerIdx);
			break;
		}
		else if(doRemove)
		{
			game.wobjects.free(this);
			break;
		}
	}
	while(w.shotType == Weapon::STLaser
	&& used // TEMP
	&& (iter < 8 || w.id == 28));
}
