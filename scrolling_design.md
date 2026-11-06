The purpose of this GlazeWM fork is to add scrolling support to the tiling window manager.

- Each desktop environment should have a scrolling system similar to PaperWM
	- Scrolling along the top bar (zebar in this case) should scroll left-and-right through the windows
	- Windows should open side-by-side rather than trying to fit vertically. Since the screen will scroll, there is no risk of running out of room.
	- Vertically stacking/unstacking windows should behave the same as PaperWM
- The desktop environment and zebar systems should stay the same and should not inherit behavior from PaperWM, with a few possible exceptions in the future.
- For the sake of testing this in parallel with GlazeWM, it should install to a separate path, so that the original GlazeWM and this fork can both be installed and compared on the user's device.
