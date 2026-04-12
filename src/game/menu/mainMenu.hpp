#ifndef LIERO_MENU_MAINMENU_HPP
#define LIERO_MENU_MAINMENU_HPP

#include "menu.hpp"

struct Common;
struct ItemBehavior;

struct MainMenu : Menu
{
	enum
	{
		MaResumeGame,
		MaNewGame,
		MaSettings,
		MaPlayer1Settings,
		MaPlayer2Settings,
		MaPlayer3Settings,
		MaPlayer4Settings,
		MaNumPlayers,
		MaAdvanced,
		MaQuit,
		MaReplays,
		MaReplay,
		MaTc
	};

	MainMenu(int x, int y)
	: Menu(x, y)
	{
	}

	virtual ItemBehavior* getItemBehavior(Common& common, MenuItem& item);
};

#endif // LIERO_MENU_MAINMENU_HPP
